pub mod plasma;

use borsh::{BorshDeserialize, BorshSerialize};
use jupiter_amm_interface::{
    AccountMap, Amm, AmmContext, KeyedAccount, Quote, QuoteParams, Swap, SwapAndAccountMetas,
    SwapParams, try_get_account_data,
};
use plasma::{PoolHeader, plasma_amm::Amm as PlasmaAmmState, plasma_utils, swap};
use solana_clock::Clock;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use spl_token_interface::state::Account as TokenAccount;
use std::sync::atomic::Ordering;

#[derive(Debug, Copy, Clone, BorshDeserialize, BorshSerialize)]
#[repr(C)]
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
        plasma_utils::ID
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
            solana_sdk_ids::sysvar::clock::id(),
        ]
    }

    fn update(&mut self, account_map: &AccountMap) -> anyhow::Result<()> {
        let base_vault_account =
            try_get_account_data(account_map, &self.plasma_amm.header.base_params.vault_key)?;
        let base_vault_token_account = TokenAccount::unpack(base_vault_account)?;

        let quote_vault_account =
            try_get_account_data(account_map, &self.plasma_amm.header.quote_params.vault_key)?;
        let quote_vault_token_account = TokenAccount::unpack(quote_vault_account)?;

        self.base_vault_amount = base_vault_token_account.amount;
        self.quote_vault_amount = quote_vault_token_account.amount;

        // Update market account
        let plasma_amm_data = try_get_account_data(account_map, &self.pool_address)?;
        let plasma_amm = PoolAccount::try_from_slice(&plasma_amm_data)?;
        self.plasma_amm = plasma_amm;

        // Update slot
        let clock_account =
            try_get_account_data(account_map, &solana_sdk_ids::sysvar::clock::id())?;
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
