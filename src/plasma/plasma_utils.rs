use borsh::{BorshDeserialize, BorshSerialize};
use bytemuck::{Pod, Zeroable};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::{Pubkey, declare_id};
use solana_sdk_ids::system_program;

declare_id!("srAMMzfVHVAtgSJc8iH6CfKzuWuUTzLHVCE81QU1rgi");

const SWAP_DISCRIMINATOR: u8 = 0;
const ADD_LIQUIDITY_DISCRIMINATOR: u8 = 1;
const REMOVE_LIQUIDITY_DISCRIMINATOR: u8 = 2;
const INITIALIZE_LP_POSITION_DISCRIMINATOR: u8 = 5;
const INITIALIZE_POOL_DISCRIMINATOR: u8 = 6;
const TRANSFER_LIQUIDITY_DISCRIMINATOR: u8 = 9;

pub const POOL_LEN: u64 = 624;
pub const POOL_DISCRIMINATOR: [u8; 8] = [116, 210, 187, 119, 196, 196, 52, 137];

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, BorshDeserialize, BorshSerialize)]
#[borsh(crate = "borsh")]
pub struct InitializePoolParams {
    pub lp_fee_in_bps: u64,
    pub protocol_fee_allocation_in_pct: u64,
    pub fee_recipients_params: [ProtocolFeeRecipientParams; 3],
    /// This is the number of slots that the LP shares will be vested over
    /// If this value is not a multiple of the leader slot window, it will be rounded down
    pub num_slots_to_vest_lp_shares: Option<u64>,
}

#[derive(Debug, Default, Copy, Clone, BorshDeserialize, BorshSerialize)]
#[repr(C)]
#[borsh(crate = "borsh")]
pub struct ProtocolFeeRecipientParams {
    pub recipient: Pubkey,
    pub shares: u64,
}

pub fn get_vault_address(plasma_program_id: &Pubkey, pool: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"vault", pool.as_ref(), mint.as_ref()],
        &plasma_program_id,
    )
}

pub fn get_lp_position_address(
    plasma_program_id: &Pubkey,
    pool: &Pubkey,
    trader: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"lp_position", pool.as_ref(), trader.as_ref()],
        &plasma_program_id,
    )
}

pub fn get_log_authority(plasma_program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"log"], plasma_program_id).0
}

fn spl_token_program_id() -> Pubkey {
    Pubkey::new_from_array(spl_token_interface::ID.to_bytes())
}

pub fn initialize_pool(
    pool_key: &Pubkey,
    pool_creator: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    params: InitializePoolParams,
) -> Instruction {
    let base_vault = get_vault_address(&ID, pool_key, base_mint).0;
    let quote_vault = get_vault_address(&ID, pool_key, quote_mint).0;
    let log_authority = get_log_authority(&ID);

    Instruction {
        program_id: ID,
        accounts: vec![
            AccountMeta::new_readonly(ID, false),
            AccountMeta::new_readonly(log_authority, false),
            AccountMeta::new(*pool_key, false),
            AccountMeta::new(*pool_creator, true),
            AccountMeta::new_readonly(*base_mint, false),
            AccountMeta::new_readonly(*quote_mint, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(spl_token_program_id(), false),
        ],
        data: [
            vec![INITIALIZE_POOL_DISCRIMINATOR],
            borsh::to_vec(&params).unwrap(),
        ]
        .concat(),
    }
}

pub fn initialize_lp_position(
    pool_key: &Pubkey,
    payer: &Pubkey,
    lp_position_owner: &Pubkey,
) -> Instruction {
    let log_authority = get_log_authority(&ID);
    let (lp_position_key, _) = get_lp_position_address(&ID, pool_key, lp_position_owner);
    Instruction {
        program_id: ID,
        accounts: vec![
            AccountMeta::new_readonly(ID, false),
            AccountMeta::new_readonly(log_authority, false),
            AccountMeta::new(*pool_key, false),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*lp_position_owner, false),
            AccountMeta::new(lp_position_key, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: vec![INITIALIZE_LP_POSITION_DISCRIMINATOR],
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, BorshDeserialize, BorshSerialize)]
#[borsh(crate = "borsh")]
pub struct AddLiquidityParams {
    pub desired_base_amount_in: u64,
    pub desired_quote_amount_in: u64,
    pub initial_lp_shares: Option<u64>,
}

pub fn add_liquidity(
    pool_key: &Pubkey,
    trader: &Pubkey,
    base_mint: &Pubkey,
    base_mint_account_key: &Pubkey,
    quote_mint: &Pubkey,
    quote_mint_account_key: &Pubkey,
    params: AddLiquidityParams,
) -> Instruction {
    let log_authority = get_log_authority(&ID);
    let (lp_position_key, _) = get_lp_position_address(&ID, pool_key, trader);

    let (base_vault_key, _) = get_vault_address(&ID, pool_key, base_mint);
    let (quote_vault_key, _) = get_vault_address(&ID, pool_key, quote_mint);

    Instruction {
        program_id: ID,
        accounts: vec![
            AccountMeta::new_readonly(ID, false),
            AccountMeta::new_readonly(log_authority, false),
            AccountMeta::new(*pool_key, false),
            AccountMeta::new_readonly(*trader, true),
            AccountMeta::new(lp_position_key, false),
            AccountMeta::new(*base_mint_account_key, false),
            AccountMeta::new(*quote_mint_account_key, false),
            AccountMeta::new(base_vault_key, false),
            AccountMeta::new(quote_vault_key, false),
            AccountMeta::new_readonly(spl_token_program_id(), false),
        ],
        data: [
            vec![ADD_LIQUIDITY_DISCRIMINATOR],
            borsh::to_vec(&params).unwrap(),
        ]
        .concat(),
    }
}

pub fn transfer_liquidity(pool_key: &Pubkey, src: &Pubkey, dst: &Pubkey) -> Instruction {
    let log_authority = get_log_authority(&ID);
    let (src_lp_position_key, _) = get_lp_position_address(&ID, pool_key, src);
    let (dst_lp_position_key, _) = get_lp_position_address(&ID, pool_key, dst);

    Instruction {
        program_id: ID,
        accounts: vec![
            AccountMeta::new_readonly(ID, false),
            AccountMeta::new_readonly(log_authority, false),
            AccountMeta::new(*pool_key, false),
            AccountMeta::new(*src, true),
            AccountMeta::new(src_lp_position_key, false),
            AccountMeta::new(dst_lp_position_key, false),
        ],
        data: vec![TRANSFER_LIQUIDITY_DISCRIMINATOR],
    }
}

pub fn remove_liquidity(
    pool_key: &Pubkey,
    trader: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    base_account_key: &Pubkey,
    quote_account_key: &Pubkey,
    shares: u64,
) -> Instruction {
    let log_authority = get_log_authority(&ID);
    let (lp_position_key, _) = get_lp_position_address(&ID, pool_key, trader);
    let base_vault_key = get_vault_address(&ID, pool_key, base_mint).0;
    let quote_vault_key = get_vault_address(&ID, pool_key, quote_mint).0;

    Instruction {
        program_id: ID,
        accounts: vec![
            AccountMeta::new_readonly(ID, false),
            AccountMeta::new_readonly(log_authority, false),
            AccountMeta::new(*pool_key, false),
            AccountMeta::new_readonly(*trader, true),
            AccountMeta::new(lp_position_key, false),
            AccountMeta::new(*base_account_key, false),
            AccountMeta::new(*quote_account_key, false),
            AccountMeta::new(base_vault_key, false),
            AccountMeta::new(quote_vault_key, false),
            AccountMeta::new_readonly(spl_token_program_id(), false),
        ],
        data: [
            vec![REMOVE_LIQUIDITY_DISCRIMINATOR],
            borsh::to_vec(&shares).unwrap(),
        ]
        .concat(),
    }
}

#[derive(Debug, Copy, Clone, BorshDeserialize, BorshSerialize)]
#[repr(C)]
#[borsh(crate = "borsh")]
pub struct LpPosition {
    reward_factor_snapshot: i128,
    pub lp_shares: u64,
    pub withdrawable_lp_shares: u64,
    uncollected_fees: u64,
    collected_fees: u64,
    pub pending_shares_to_vest: (u64, u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshDeserialize, BorshSerialize)]
#[borsh(crate = "borsh")]
pub enum Side {
    Buy,
    Sell,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, BorshDeserialize, BorshSerialize)]
#[borsh(crate = "borsh")]
pub struct SwapParams {
    pub side: Side,
    pub swap_type: SwapType,
}

#[derive(Clone, Copy, Debug, BorshDeserialize, BorshSerialize)]
#[borsh(crate = "borsh")]
pub enum SwapType {
    ExactIn { amount_in: u64, min_amount_out: u64 },
    ExactOut { amount_out: u64, max_amount_in: u64 },
}

pub fn swap(
    pool_key: &Pubkey,
    trader: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    base_account_key: &Pubkey,
    quote_account_key: &Pubkey,
    params: SwapParams,
) -> Instruction {
    let log_authority = get_log_authority(&ID);
    let base_vault_key = get_vault_address(&ID, pool_key, base_mint).0;
    let quote_vault_key = get_vault_address(&ID, pool_key, quote_mint).0;

    Instruction {
        program_id: ID,
        accounts: vec![
            AccountMeta::new_readonly(ID, false),
            AccountMeta::new_readonly(log_authority, false),
            AccountMeta::new(*pool_key, false),
            AccountMeta::new_readonly(*trader, true),
            AccountMeta::new(*base_account_key, false),
            AccountMeta::new(*quote_account_key, false),
            AccountMeta::new(base_vault_key, false),
            AccountMeta::new(quote_vault_key, false),
            AccountMeta::new_readonly(spl_token_program_id(), false),
        ],
        data: [vec![SWAP_DISCRIMINATOR], borsh::to_vec(&params).unwrap()].concat(),
    }
}

#[derive(Debug, Copy, Clone, Zeroable, Pod, BorshDeserialize, BorshSerialize)]
#[repr(C)]
#[borsh(crate = "borsh")]
pub struct PoolHeader {
    pub discriminator: [u8; 8],
    pub sequence_number: u64,
    pub base_params: TokenParams,
    pub quote_params: TokenParams,
    pub fee_recipients: ProtocolFeeRecipients,
    pub swap_sequence_number: u64,
    pub padding: [u64; 12],
}

#[derive(Debug, Copy, Clone, Zeroable, Pod, BorshDeserialize, BorshSerialize)]
#[repr(C)]
#[borsh(crate = "borsh")]
pub struct TokenParams {
    /// Number of decimals for the token (e.g. 9 for SOL, 6 for USDC).
    pub decimals: u32,

    /// Bump used for generating the PDA for the pool's token vault.
    pub vault_bump: u32,

    /// Pubkey of the token mint.
    pub mint_key: Pubkey,

    /// Pubkey of the token vault.
    pub vault_key: Pubkey,
}

#[derive(Debug, Default, Copy, Clone, Zeroable, Pod, BorshDeserialize, BorshSerialize)]
#[repr(C)]
#[borsh(crate = "borsh")]
pub struct ProtocolFeeRecipient {
    pub recipient: Pubkey,
    pub shares: u64,
    pub total_accumulated_quote_fees: u64,
    pub collected_quote_fees: u64,
}

#[derive(Debug, Default, Copy, Clone, Zeroable, Pod, BorshDeserialize, BorshSerialize)]
#[repr(C)]
#[borsh(crate = "borsh")]
pub struct ProtocolFeeRecipients {
    pub recipients: [ProtocolFeeRecipient; 3],
    _padding: [u64; 12],
}
