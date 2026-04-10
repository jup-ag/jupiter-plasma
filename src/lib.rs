pub mod plasma;

use borsh::{BorshDeserialize, BorshSerialize};
use jupiter_amm_interface::{
    AccountMap, Amm, AmmContext, KeyedAccount, Quote, QuoteParams, Swap, SwapAndAccountMetas,
    SwapParams, try_get_account_data,
};
use plasma::{PoolHeader, plasma_amm::Amm as PlasmaAmmState, plasma_utils, swap};
use solana_clock::Clock;
use solana_pubkey::Pubkey;
use solana_sdk_ids::sysvar;
use std::sync::atomic::Ordering;

#[derive(Debug, Copy, Clone, BorshDeserialize, BorshSerialize)]
#[repr(C)]
#[borsh(crate = "borsh")]
pub struct PoolAccount {
    pub header: PoolHeader,
    pub amm: PlasmaAmmState,
}

#[derive(Clone)]
pub struct PlasmaAmm {
    pub pool_address: Pubkey,
    pub plasma_amm: PoolAccount,
    pub base_vault_amount: u64,
    pub quote_vault_amount: u64,
    pub slot: u64,
}

impl Amm for PlasmaAmm {
    fn label(&self) -> String {
        "Plasma".to_string()
    }

    fn program_id(&self) -> Pubkey {
        plasma_utils::id()
    }

    fn key(&self) -> Pubkey {
        self.pool_address
    }

    fn get_reserve_mints(&self) -> Vec<Pubkey> {
        vec![
            self.plasma_amm.header.base_params.mint_key,
            self.plasma_amm.header.quote_params.mint_key,
        ]
    }

    fn get_accounts_to_update(&self) -> Vec<Pubkey> {
        vec![
            self.pool_address,
            self.plasma_amm.header.base_params.vault_key,
            self.plasma_amm.header.quote_params.vault_key,
            sysvar::clock::id(),
        ]
    }

    fn update(&mut self, account_map: &AccountMap) -> anyhow::Result<()> {
        let base_vault_account =
            try_get_account_data(account_map, &self.plasma_amm.header.base_params.vault_key)?;
        let base_vault_amount = token_amount(base_vault_account)?;

        let quote_vault_account =
            try_get_account_data(account_map, &self.plasma_amm.header.quote_params.vault_key)?;
        let quote_vault_amount = token_amount(quote_vault_account)?;

        self.base_vault_amount = base_vault_amount;
        self.quote_vault_amount = quote_vault_amount;

        // Update market account
        let plasma_amm_data = try_get_account_data(account_map, &self.pool_address)?;
        let plasma_amm = PoolAccount::try_from_slice(&plasma_amm_data)?;
        self.plasma_amm = plasma_amm;

        // Update slot
        let clock_account = try_get_account_data(account_map, &sysvar::clock::id())?;
        let clock = bincode::deserialize::<Clock>(clock_account)?;
        self.slot = clock.slot;

        Ok(())
    }

    fn quote(&self, quote_params: &QuoteParams) -> anyhow::Result<Quote> {
        let QuoteParams {
            amount,
            input_mint,
            output_mint: _output_mint,
            swap_mode: _swap_mode,
            ..
        } = quote_params;

        let is_quote_to_base = input_mint == &self.plasma_amm.header.quote_params.mint_key;

        let swap_result = if is_quote_to_base {
            self.plasma_amm
                .amm
                .simulate_buy_exact_in_with_slot(self.slot, *amount)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
        } else {
            self.plasma_amm
                .amm
                .simulate_sell_exact_in_with_slot(self.slot, *amount)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
        };

        let out_amount = if is_quote_to_base {
            swap_result.base_amount_to_transfer
        } else {
            swap_result.quote_amount_to_transfer
        };

        Ok(Quote {
            in_amount: *amount,
            out_amount,
            ..Quote::default()
        })
    }

    fn get_swap_and_account_metas(
        &self,
        swap_params: &SwapParams,
    ) -> anyhow::Result<SwapAndAccountMetas> {
        let SwapParams {
            in_amount,
            out_amount,
            source_mint,
            destination_mint: _destination_mint,
            source_token_account,
            destination_token_account,
            token_transfer_authority,
            ..
        } = swap_params;

        let (user_base_ata, user_quote_ata) =
            if *source_mint == self.plasma_amm.header.base_params.mint_key {
                (source_token_account, destination_token_account)
            } else {
                (destination_token_account, source_token_account)
            };

        let side = if *source_mint == self.plasma_amm.header.quote_params.mint_key {
            plasma_utils::Side::Buy
        } else {
            plasma_utils::Side::Sell
        };

        // FIXME: ADD THIS STRUCT TO `swap: ` BELOW
        let swap_instruction_data = plasma_utils::SwapParams {
            side,
            swap_type: plasma_utils::SwapType::ExactIn {
                amount_in: *in_amount,
                min_amount_out: *out_amount,
            },
        };

        let swap_ix = swap(
            &self.pool_address,
            token_transfer_authority,
            &self.plasma_amm.header.base_params.mint_key,
            &self.plasma_amm.header.quote_params.mint_key,
            user_base_ata,
            user_quote_ata,
            swap_instruction_data,
        );

        Ok(SwapAndAccountMetas {
            swap: Swap::TokenSwap, // FIXME: REPLACE WITH STUCT ABOVE ONCE WE'RE ADDED TO THE ENUM
            account_metas: swap_ix.accounts,
        })
    }

    fn get_accounts_len(&self) -> usize {
        9
    }

    fn clone_amm(&self) -> Box<dyn Amm + Send + Sync> {
        Box::new(self.clone())
    }

    fn from_keyed_account(
        keyed_account: &KeyedAccount,
        amm_context: &AmmContext,
    ) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let pool_account = PoolAccount::try_from_slice(&keyed_account.account.data)?;
        let pool_address = keyed_account.key;

        Ok(Self {
            pool_address,
            plasma_amm: pool_account,
            base_vault_amount: 0,
            quote_vault_amount: 0,
            slot: amm_context.clock_ref.slot.load(Ordering::Relaxed),
        })
    }
}

fn token_amount(data: &[u8]) -> anyhow::Result<u64> {
    const TOKEN_ACCOUNT_LEN: usize = 165;
    const AMOUNT_OFFSET: usize = 64;
    const AMOUNT_END: usize = AMOUNT_OFFSET + 8;

    if data.len() < TOKEN_ACCOUNT_LEN {
        anyhow::bail!("invalid token account length: {}", data.len());
    }

    let bytes: [u8; 8] = data[AMOUNT_OFFSET..AMOUNT_END]
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid token amount bytes"))?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use jupiter_amm_interface::{ClockRef, SwapMode};
    use solana_account::Account;
    use solana_client::rpc_client::RpcClient;
    use solana_commitment_config::CommitmentConfig;
    use solana_pubkey::pubkey;

    use super::*;

    #[test]
    fn test_plasma_amm() {
        let rpc_url = "https://api.mainnet-beta.solana.com".to_string();
        let client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

        let pool_pubkey = pubkey!("8aDt3G915nUwxrqoN4tsBT4SgYwSsE5K2mBfu65JY4ex");

        let clock_account = client.get_account(&sysvar::clock::id()).unwrap();
        let clock = bincode::deserialize::<Clock>(clock_account.data.as_slice()).unwrap();

        let clock_ref = ClockRef::default();
        clock_ref.update(clock);

        let amm_context = AmmContext { clock_ref };

        let account = client.get_account(&pool_pubkey).unwrap();

        let pool_account = KeyedAccount {
            key: pool_pubkey,
            account,
            params: None,
        };

        let mut plasma_amm = PlasmaAmm::from_keyed_account(&pool_account, &amm_context).unwrap();

        let accounts_to_update = plasma_amm.get_accounts_to_update();

        let accounts_map = client
            .get_multiple_accounts(&accounts_to_update)
            .unwrap()
            .iter()
            .enumerate()
            .fold(
                HashMap::<Pubkey, Account, ahash::RandomState>::default(),
                |mut m, (index, account)| {
                    if let Some(account) = account {
                        m.insert(accounts_to_update[index], account.clone());
                    }
                    m
                },
            );
        plasma_amm.update(&accounts_map).unwrap();
        println!("Buying with 1 SOL");
        let res = plasma_amm
            .quote(&QuoteParams {
                amount: 1e9 as u64,
                input_mint: plasma_amm.plasma_amm.header.quote_params.mint_key,
                output_mint: plasma_amm.plasma_amm.header.base_params.mint_key,
                swap_mode: SwapMode::ExactIn,
                fee_mode: jupiter_amm_interface::FeeMode::Normal,
            })
            .unwrap();

        println!("Received {:?} Tokens", res.out_amount as f64 / 1e6);

        println!("Selling with {} Tokens", res.out_amount as f64 / 1e6);

        let res = plasma_amm
            .quote(&QuoteParams {
                amount: res.out_amount as u64,
                input_mint: plasma_amm.plasma_amm.header.base_params.mint_key,
                output_mint: plasma_amm.plasma_amm.header.quote_params.mint_key,
                swap_mode: SwapMode::ExactIn,
                fee_mode: jupiter_amm_interface::FeeMode::Normal,
            })
            .unwrap();
        println!("Received {:?} SOL", res.out_amount as f64 / 1e9);
    }
}
