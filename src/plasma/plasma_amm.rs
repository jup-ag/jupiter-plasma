use borsh::{BorshDeserialize, BorshSerialize};
use bytemuck::{Pod, Zeroable};

use crate::plasma::{Downcast, PlasmaStateError, Upcast, fixed::I80F48};

pub const BPS_BASE: u128 = 10000;

use super::SlotWindow;

#[derive(Debug, Clone, Copy, BorshDeserialize, BorshSerialize)]
pub struct SwapResult {
    pub side: Side,
    pub base_amount_to_transfer: u64,
    pub quote_amount_to_transfer: u64,
    pub base_matched_as_limit_order: u64,
    pub quote_matched_as_limit_order: u64,
    pub base_matched_as_swap: u64,
    pub quote_matched_as_swap: u64,
    pub fee_in_quote: u64,
}

impl SwapResult {
    fn new_empty_with_side(side: Side) -> Self {
        Self {
            side,
            base_amount_to_transfer: 0,
            quote_amount_to_transfer: 0,
            base_matched_as_limit_order: 0,
            quote_matched_as_limit_order: 0,
            base_matched_as_swap: 0,
            quote_matched_as_swap: 0,
            fee_in_quote: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshDeserialize, BorshSerialize)]
pub enum Side {
    Buy,
    Sell,
}

/// Enum to differentiate between base and quote tokens
pub enum TokenType {
    Base,
    Quote,
}

#[derive(Debug, Copy, Clone, Zeroable, Pod, BorshDeserialize, BorshSerialize)]
#[repr(C)]
pub struct Amm {
    pub fee_in_bps: u32,
    protocol_allocation_in_pct: u32,
    pub lp_vesting_window: u64,
    pub reward_factor: I80F48,
    pub total_lp_shares: u64,
    slot_snapshot: u64,
    pub base_reserves_snapshot: u64,
    pub quote_reserves_snapshot: u64,
    pub base_reserves: u64,
    pub quote_reserves: u64,
    pub cumulative_quote_lp_fees: u64,
    pub cumulative_quote_protocol_fees: u64,
}

impl Amm {
    pub fn new(
        fee_in_bps: u32,
        protocol_allocation_in_pct: u32,
        lp_vesting_window: u64,
        slot_snapshot: u64,
    ) -> Self {
        Self {
            fee_in_bps,
            protocol_allocation_in_pct,
            lp_vesting_window,
            reward_factor: I80F48::ZERO,
            total_lp_shares: 0,
            slot_snapshot,
            base_reserves_snapshot: 0,
            quote_reserves_snapshot: 0,
            base_reserves: 0,
            quote_reserves: 0,
            cumulative_quote_lp_fees: 0,
            cumulative_quote_protocol_fees: 0,
        }
    }
}

impl Amm {
    pub fn simulate_buy_exact_in(&self, quote_in: u64) -> Result<SwapResult, PlasmaStateError> {
        self.simulate_buy_exact_in_with_slot(self.get_slot(), quote_in)
    }

    pub fn simulate_sell_exact_in(&self, base_in: u64) -> Result<SwapResult, PlasmaStateError> {
        self.simulate_sell_exact_in_with_slot(self.get_slot(), base_in)
    }

    pub fn simulate_buy_exact_in_with_slot(
        &self,
        slot: SlotWindow,
        quote_in: u64,
    ) -> Result<SwapResult, PlasmaStateError> {
        let mut pool_clone = self.clone();
        pool_clone.buy_exact_in(slot, quote_in)
    }

    pub fn simulate_sell_exact_in_with_slot(
        &self,
        slot: SlotWindow,
        base_in: u64,
    ) -> Result<SwapResult, PlasmaStateError> {
        let mut pool_clone = self.clone();
        pool_clone.sell_exact_in(slot, base_in)
    }
}

impl Amm {
    pub fn get_slot(&self) -> SlotWindow {
        self.slot_snapshot
    }

    pub fn deposit_amount_quote(&self, amount_base: u64) -> u128 {
        amount_base.upcast() * self.quote_reserves.upcast() / self.base_reserves.upcast()
    }

    pub fn deposit_amount_base(&self, amount_quote: u64) -> u128 {
        amount_quote.upcast() * self.base_reserves.upcast() / self.quote_reserves.upcast()
    }
}

pub struct LimitOrderConfiguration {
    size_in_base: u128,
    size_in_quote: u128,
}

impl LimitOrderConfiguration {
    fn new_default() -> Self {
        Self {
            size_in_base: 0,
            size_in_quote: 0,
        }
    }
}

impl Amm {
    /// This function solves the closed-form solution for the size of the virtual limit order
    /// in the pool. The virutal limit order is always priced at the snapshot price.
    ///
    /// The size of the limit order is determined by the following constraint:
    ///
    /// ```ignore
    /// (quote_snapshot / base_snapshot) = (quote_reserves + ∆_quote) / (base_reserves + ∆_base)
    /// ```
    ///
    /// Note that the signs of ∆_quote and ∆_base are always flipped.
    ///
    /// This means that the size of the limit order is set such that the new pool price
    /// after the swap is equal to the price at the snapshot.
    ///
    /// Because we know the limit order is priced at the snapshot price, we can derive
    /// the following equations:
    /// -  ∆_base = -∆_quote * base_snapshot / quote_snapshot
    /// -  ∆_quote = -∆_base * quote_snapshot / base_snapshot
    ///
    ///
    /// We can then solve for ∆_base and ∆_quote after substituting the above equations. There are separate cases
    /// for buy and sell
    ///
    /// - Limit order on the buy side (bid)
    /// ```ignore
    /// ∆_base = (base_snapshot * quote_reserves - quote_snapshot * base_reserves) / (2 * quote_snapshot)
    /// ∆_quote = (base_snapshot * quote_reserves - quote_snapshot * base_reserves) / (2 * base_snapshot)
    /// ```
    ///
    /// - Limit order on the sell side (ask)
    /// ```ignore
    /// ∆_base = (quote_snapshot * base_reserves - base_snapshot * quote_reserves) / (2 * quote_snapshot)
    /// ∆_quote = (quote_snapshot * base_reserves - base_snapshot * quote_reserves) / (2 * base_snapshot)
    /// ```
    ///
    pub fn get_limit_order_size_in_base_and_quote(
        &self,
        side: Side,
    ) -> Option<LimitOrderConfiguration> {
        let quote_snapshot = self.quote_reserves_snapshot.upcast();
        let base_snapshot = self.base_reserves_snapshot.upcast();
        let quote_reserves = self.quote_reserves.upcast();
        let base_reserves = self.base_reserves.upcast();

        match side {
            Side::Buy => {
                let ask = if quote_snapshot.checked_mul(base_reserves)?
                    > base_snapshot.checked_mul(quote_reserves)?
                {
                    let size_in_quote = quote_snapshot
                        .checked_mul(base_reserves)?
                        .checked_sub(base_snapshot.checked_mul(quote_reserves)?)?
                        .checked_div(2u128.checked_mul(base_snapshot)?)?;

                    let size_in_base = size_in_quote
                        .checked_mul(base_snapshot)?
                        .checked_div(quote_snapshot)?;

                    LimitOrderConfiguration {
                        size_in_base,
                        size_in_quote,
                    }
                } else {
                    LimitOrderConfiguration::new_default()
                };
                Some(ask)
            }
            Side::Sell => {
                let bid = if base_snapshot.checked_mul(quote_reserves)?
                    > quote_snapshot.checked_mul(base_reserves)?
                {
                    let size_in_base = base_snapshot
                        .checked_mul(quote_reserves)?
                        .checked_sub(quote_snapshot.checked_mul(base_reserves)?)?
                        .checked_div(2u128.checked_mul(quote_snapshot)?)?;

                    let size_in_quote = size_in_base
                        .checked_mul(quote_snapshot)?
                        .checked_div(base_snapshot)?;

                    LimitOrderConfiguration {
                        size_in_base,
                        size_in_quote,
                    }
                } else {
                    LimitOrderConfiguration::new_default()
                };
                Some(bid)
            }
        }
    }

    /// This function returns the size of the virtual limit order in the complementary token type
    /// given an `amount` and the `input_token_type`.
    ///  - If the `input_token_type` is Base, then the size of the limit order in Quote is computed.
    ///  - If the `input_token_type` is Quote, then the size of the limit order in Base is computed.
    fn get_complementary_limit_order_size(
        &self,
        // This amount is in the token type specified by `input_token_type`
        amount: u128,
        side: Side,
        input_token_type: TokenType,
    ) -> Option<u128> {
        if amount == 0 {
            return Some(0);
        }
        let quote_snapshot = self.quote_reserves_snapshot.upcast();
        let base_snapshot = self.base_reserves_snapshot.upcast();
        match side {
            Side::Buy => {
                match input_token_type {
                    // If `amount` is in base, then the size of the limit order in quote is computed and rounded up
                    TokenType::Base => Some(
                        amount
                            .checked_mul(quote_snapshot)?
                            .saturating_sub(1)
                            .checked_div(base_snapshot)?
                            .saturating_add(1),
                    ),
                    // If `amount` is in quote, then the size of the limit order in base is computed
                    TokenType::Quote => amount
                        .checked_mul(base_snapshot)?
                        .checked_div(quote_snapshot),
                }
            }
            Side::Sell => {
                match input_token_type {
                    // If `amount` is in base, then the size of the limit order in quote is computed
                    TokenType::Base => amount
                        .checked_mul(quote_snapshot)?
                        .checked_div(base_snapshot),
                    // If `amount` is in quote, then the size of the limit order in base is computed and rounded up
                    TokenType::Quote => Some(
                        amount
                            .checked_mul(base_snapshot)?
                            .saturating_sub(1)
                            .checked_div(quote_snapshot)?
                            .saturating_add(1),
                    ),
                }
            }
        }
    }
}

impl Amm {
    pub fn get_base_out_from_quote_in(&self, quote_in: u128) -> Option<u128> {
        let base_reserves = self.base_reserves.upcast();
        let quote_reserves = self.quote_reserves.upcast();
        let k = base_reserves.checked_mul(quote_reserves)?.saturating_sub(1);
        let base_out = base_reserves.checked_sub(
            k.checked_div(quote_reserves.checked_add(quote_in)?)?
                .saturating_add(1),
        );
        base_out
    }

    pub fn get_quote_in_from_base_out(&self, base_out: u128) -> Result<u128, PlasmaStateError> {
        let base_reserves = self.base_reserves.upcast();
        let quote_reserves = self.quote_reserves.upcast();
        let k = base_reserves
            .checked_mul(quote_reserves)
            .ok_or(PlasmaStateError::Overflow)?
            .saturating_sub(1);
        if base_out >= base_reserves {
            return Err(PlasmaStateError::SwapOutputGreaterThanOrEqualToReserves(
                base_out,
                base_reserves,
            ));
        }
        let quote_in = (k / (base_reserves - base_out)).saturating_add(1) - quote_reserves;
        Ok(quote_in)
    }

    pub fn get_quote_out_from_base_in(&self, base_in: u128) -> u128 {
        let base_reserves = self.base_reserves.upcast();
        let quote_reserves = self.quote_reserves.upcast();
        let k = (base_reserves * quote_reserves).saturating_sub(1);
        let quote_out = quote_reserves - (k / (base_reserves + base_in)).saturating_add(1);
        quote_out
    }

    pub fn get_base_in_from_quote_out(&self, quote_out: u128) -> Result<u128, PlasmaStateError> {
        let base_reserves = self.base_reserves.upcast();
        let quote_reserves = self.quote_reserves.upcast();
        let k = (base_reserves * quote_reserves).saturating_sub(1);
        if quote_out >= quote_reserves {
            return Err(PlasmaStateError::SwapOutputGreaterThanOrEqualToReserves(
                quote_out,
                quote_reserves,
            ));
        }
        let base_in = (k / (quote_reserves - quote_out)).saturating_add(1) - base_reserves;
        Ok(base_in)
    }
}

impl Amm {
    fn get_fee_splits(&self, total_fees: u64) -> (u64, u64) {
        // This will round down so LPs get any remainders
        let protocol_fees = (total_fees * self.protocol_allocation_in_pct as u64) / 100;
        let lp_fees = total_fees - protocol_fees;
        (lp_fees, protocol_fees)
    }

    pub fn fee_rounded_down(&self, amount: u128) -> Option<u128> {
        amount
            .checked_mul(self.fee_in_bps.upcast())?
            .checked_div(10_000)
    }

    pub fn pre_fee_adjust_rounded_down(&self, amount: u128) -> Option<u128> {
        // x * 10000 / (10000 - fee) is approximately equivalent to x * (1 - fee / 10000)
        let numerator = amount.checked_mul(BPS_BASE)?;
        let denominator = BPS_BASE.checked_sub(self.fee_in_bps.upcast())?;
        numerator.checked_div(denominator)
    }
}

impl Amm {
    pub fn maybe_update_snapshot(&mut self, new_snapshot_slot: SlotWindow) -> bool {
        if new_snapshot_slot > self.slot_snapshot {
            self.slot_snapshot = new_snapshot_slot;
            self.base_reserves_snapshot = self.base_reserves;
            self.quote_reserves_snapshot = self.quote_reserves;
            true
        } else {
            false
        }
    }
}

impl Amm {
    pub fn mint(
        &mut self,
        slot: SlotWindow,
        base_amount_desired: u64,
        quote_amount_desired: u64,
        initial_lp_shares: Option<u64>,
    ) -> Result<(u64, u64, u64), PlasmaStateError> {
        self.maybe_update_snapshot(slot);

        let total_shares = self.total_lp_shares.upcast();
        let (base_amount_deposited, quote_amount_deposited, lp_shares) = if total_shares == 0 {
            let Some(lp_shares) = initial_lp_shares.map(|s| s.upcast()) else {
                return Err(PlasmaStateError::MissingExpectedArgument);
            };
            let initial_k = base_amount_desired.upcast() * quote_amount_desired.upcast();
            let lp_shares_squared = lp_shares * lp_shares;

            // Check that lp_shares^2 <= initial_k < (lp_shares + 1)^2
            if lp_shares_squared <= initial_k && lp_shares_squared + (lp_shares * 2) + 1 > initial_k
            {
                // On initial deposit, set up the pool snapshot
                self.base_reserves_snapshot = base_amount_desired;
                self.quote_reserves_snapshot = quote_amount_desired;

                // Set the AMM reserves
                self.base_reserves = base_amount_desired;
                self.quote_reserves = quote_amount_desired;
                (
                    base_amount_desired,
                    quote_amount_desired,
                    lp_shares.downcast()?,
                )
            } else {
                return Err(PlasmaStateError::UnexpectedArgument);
            }
        } else {
            let base_amount_optimal = self.deposit_amount_base(quote_amount_desired);
            let quote_amount_optimal = self.deposit_amount_quote(base_amount_desired);

            let base_amount_desired = base_amount_desired.upcast();
            let quote_amount_desired = quote_amount_desired.upcast();

            let (base_amount_deposited, quote_amount_deposited) =
                if quote_amount_desired >= quote_amount_optimal {
                    (base_amount_desired, quote_amount_optimal)
                } else {
                    assert!(base_amount_desired >= base_amount_optimal);
                    (base_amount_optimal, quote_amount_desired)
                };

            if initial_lp_shares.is_some() {
                return Err(PlasmaStateError::UnexpectedArgument);
            }

            let total_base = self.base_reserves.upcast();
            let total_quote = self.quote_reserves.upcast();

            // Update the AMM balances
            self.base_reserves += base_amount_deposited.downcast()?;
            self.quote_reserves += quote_amount_deposited.downcast()?;
            (
                base_amount_deposited.downcast()?,
                quote_amount_deposited.downcast()?,
                (quote_amount_deposited * total_shares / total_quote)
                    .min(base_amount_deposited * total_shares / total_base)
                    .downcast()?,
            )
        };

        if lp_shares == 0 {
            return Err(PlasmaStateError::BelowMinimumLpSharesRequired);
        }

        // Increase the total LP shares
        self.total_lp_shares += lp_shares;

        Ok((base_amount_deposited, quote_amount_deposited, lp_shares))
    }

    pub fn burn(
        &mut self,
        slot: SlotWindow,
        lp_shares: u64,
    ) -> Result<(u64, u64), PlasmaStateError> {
        self.maybe_update_snapshot(slot);
        let base_amount_withdrawn =
            self.base_reserves.upcast() * lp_shares.upcast() / self.total_lp_shares.upcast();
        let quote_amount_withdrawn =
            self.quote_reserves.upcast() * lp_shares.upcast() / self.total_lp_shares.upcast();

        if base_amount_withdrawn == 0 || quote_amount_withdrawn == 0 {
            return Err(PlasmaStateError::BelowMinimumWithdrawaRequired {
                base_amount_to_withdraw: base_amount_withdrawn.downcast()?,
                quote_amount_to_withdraw: quote_amount_withdrawn.downcast()?,
            });
        }

        self.base_reserves -= base_amount_withdrawn.downcast()?;
        self.quote_reserves -= quote_amount_withdrawn.downcast()?;
        self.total_lp_shares -= lp_shares;

        Ok((
            base_amount_withdrawn.downcast()?,
            quote_amount_withdrawn.downcast()?,
        ))
    }
}

impl Amm {
    fn update_pool_reserves_after_buy(
        &mut self,
        quote_in: u128,
        base_out: u128,
    ) -> Result<(), PlasmaStateError> {
        self.base_reserves = self
            .base_reserves
            .checked_sub(base_out.downcast()?)
            .ok_or(PlasmaStateError::Underflow)?;
        self.quote_reserves = self
            .quote_reserves
            .checked_add(quote_in.downcast()?)
            .ok_or(PlasmaStateError::Overflow)?;
        Ok(())
    }

    fn update_pool_reserves_after_sell(
        &mut self,
        base_in: u128,
        quote_out: u128,
    ) -> Result<(), PlasmaStateError> {
        self.base_reserves = self
            .base_reserves
            .checked_add(base_in.downcast()?)
            .ok_or(PlasmaStateError::Underflow)?;
        self.quote_reserves = self
            .quote_reserves
            .checked_sub(quote_out.downcast()?)
            .ok_or(PlasmaStateError::Overflow)?;
        Ok(())
    }

    fn apply_fees(&mut self, quote_fee: u128) -> Result<(), PlasmaStateError> {
        let total_fees = quote_fee.downcast()?;
        let (lp_fees, protocol_fees) = self.get_fee_splits(total_fees);
        if lp_fees + protocol_fees != total_fees {
            return Err(PlasmaStateError::MismatchedFees(
                total_fees as u128,
                (lp_fees + protocol_fees) as u128,
            ));
        }
        self.cumulative_quote_lp_fees += lp_fees;
        self.cumulative_quote_protocol_fees += protocol_fees;
        self.reward_factor += I80F48::from_fraction(lp_fees, self.total_lp_shares);
        Ok(())
    }
}

impl Amm {
    pub fn buy_exact_in(
        &mut self,
        slot: SlotWindow,
        quote_in: u64,
    ) -> Result<SwapResult, PlasmaStateError> {
        if self.total_lp_shares == 0 {
            return Err(PlasmaStateError::UninitializedPool);
        }
        self.maybe_update_snapshot(slot);

        if quote_in == 0 {
            return Ok(SwapResult::new_empty_with_side(Side::Buy));
        }

        let quote_fee = self
            .fee_rounded_down(quote_in.upcast())
            .ok_or(PlasmaStateError::Overflow)?;
        let quote_in_post_fee: u128 = quote_in
            .upcast()
            .checked_sub(quote_fee)
            .ok_or(PlasmaStateError::Underflow)?;

        let quote_reserves = self.quote_reserves.upcast();
        let base_reserves = self.base_reserves.upcast();
        let k_start = quote_reserves
            .checked_mul(base_reserves)
            .ok_or(PlasmaStateError::Overflow)?;

        let LimitOrderConfiguration {
            size_in_base: size_on_ask_in_base,
            size_in_quote: size_on_ask_in_quote,
        } = self
            .get_limit_order_size_in_base_and_quote(Side::Buy)
            .ok_or(PlasmaStateError::Overflow)?;

        let (
            quote_swapped_through_ask,
            base_swapped_through_ask,
            quote_swapped_through_pool,
            base_swapped_through_pool,
        ) = if size_on_ask_in_quote >= quote_in_post_fee {
            let quote_swapped_through_ask = quote_in_post_fee;
            let base_swapped_through_ask = self
                .get_complementary_limit_order_size(quote_in_post_fee, Side::Buy, TokenType::Quote)
                .ok_or(PlasmaStateError::Overflow)?;

            self.update_pool_reserves_after_buy(
                quote_swapped_through_ask,
                base_swapped_through_ask,
            )?;

            let quote_swapped_through_pool = 0_u128;
            let base_swapped_through_pool = 0_u128;
            (
                quote_swapped_through_ask,
                base_swapped_through_ask,
                quote_swapped_through_pool,
                base_swapped_through_pool,
            )
        } else {
            let base_swapped_through_ask = size_on_ask_in_base;
            let quote_swapped_through_ask = size_on_ask_in_quote;

            self.update_pool_reserves_after_buy(
                quote_swapped_through_ask,
                base_swapped_through_ask,
            )?;

            let quote_swapped_through_pool = quote_in_post_fee - size_on_ask_in_quote;
            let base_swapped_through_pool = self
                .get_base_out_from_quote_in(quote_swapped_through_pool)
                .ok_or(PlasmaStateError::Overflow)?;

            self.update_pool_reserves_after_buy(
                quote_swapped_through_pool,
                base_swapped_through_pool,
            )?;

            (
                quote_swapped_through_ask,
                base_swapped_through_ask,
                quote_swapped_through_pool,
                base_swapped_through_pool,
            )
        };

        let base_out = base_swapped_through_ask
            .checked_add(base_swapped_through_pool)
            .ok_or(PlasmaStateError::Overflow)?;

        let updated_base_reserves = self.base_reserves.upcast();
        let updated_quote_reserves = self.quote_reserves.upcast();

        let swap_result = SwapResult {
            side: Side::Buy,
            base_amount_to_transfer: base_out.downcast()?,
            quote_amount_to_transfer: quote_in,
            base_matched_as_limit_order: base_swapped_through_ask.downcast()?,
            quote_matched_as_limit_order: quote_swapped_through_ask.downcast()?,
            base_matched_as_swap: base_swapped_through_pool.downcast()?,
            quote_matched_as_swap: quote_swapped_through_pool.downcast()?,
            fee_in_quote: quote_fee.downcast()?,
        };

        let k_end = updated_base_reserves
            .checked_mul(updated_quote_reserves)
            .ok_or(PlasmaStateError::Overflow)?;
        if k_start > k_end {
            return Err(PlasmaStateError::InvariantViolation(k_start, k_end));
        }

        if swap_result.base_amount_to_transfer
            != swap_result
                .base_matched_as_limit_order
                .checked_add(swap_result.base_matched_as_swap)
                .ok_or(PlasmaStateError::Overflow)?
        {
            return Err(PlasmaStateError::SwapAmountMismatch);
        }
        if swap_result.quote_amount_to_transfer
            != swap_result
                .quote_matched_as_limit_order
                .checked_add(swap_result.quote_matched_as_swap)
                .and_then(|r| r.checked_add(swap_result.fee_in_quote))
                .ok_or(PlasmaStateError::Overflow)?
        {
            return Err(PlasmaStateError::SwapAmountMismatch);
        }
        if swap_result.quote_amount_to_transfer != quote_in {
            return Err(PlasmaStateError::SwapAmountMismatch);
        }

        // Apply fees, not required to mutate
        // self.apply_fees(quote_fee)?;

        Ok(swap_result)
    }

    pub fn buy_exact_out(
        &mut self,
        slot: SlotWindow,
        base_out: u64,
    ) -> Result<SwapResult, PlasmaStateError> {
        if self.total_lp_shares == 0 {
            return Err(PlasmaStateError::UninitializedPool);
        }

        if self.base_reserves < base_out {
            return Err(PlasmaStateError::SwapExactOutTooLarge);
        }

        self.maybe_update_snapshot(slot);
        if base_out == 0 {
            return Ok(SwapResult::new_empty_with_side(Side::Buy));
        }

        let base_out = base_out.upcast();
        let quote_reserves = self.quote_reserves.upcast();
        let base_reserves = self.base_reserves.upcast();
        let k_start = quote_reserves * base_reserves;

        let LimitOrderConfiguration {
            size_in_base: size_on_ask_in_base,
            size_in_quote: size_on_ask_in_quote,
        } = self
            .get_limit_order_size_in_base_and_quote(Side::Buy)
            .ok_or(PlasmaStateError::Overflow)?;

        let (
            base_swapped_through_ask,
            quote_swapped_through_ask,
            base_swapped_through_pool,
            quote_swapped_through_pool,
        ) = if size_on_ask_in_base >= base_out {
            let base_swapped_through_ask = base_out;
            let quote_swapped_through_ask = self
                .get_complementary_limit_order_size(
                    base_swapped_through_ask,
                    Side::Buy,
                    TokenType::Base,
                )
                .ok_or(PlasmaStateError::Overflow)?;

            self.update_pool_reserves_after_buy(
                quote_swapped_through_ask,
                base_swapped_through_ask,
            )?;

            let base_swapped_through_pool = 0_u128;
            let quote_swapped_through_pool = 0_u128;
            (
                base_swapped_through_ask,
                quote_swapped_through_ask,
                base_swapped_through_pool,
                quote_swapped_through_pool,
            )
        } else {
            let base_swapped_through_ask = size_on_ask_in_base;
            let quote_swapped_through_ask = size_on_ask_in_quote;

            self.update_pool_reserves_after_buy(
                quote_swapped_through_ask,
                base_swapped_through_ask,
            )?;

            let base_swapped_through_pool = base_out - size_on_ask_in_base;
            let quote_swapped_through_pool =
                self.get_quote_in_from_base_out(base_swapped_through_pool)?;

            self.update_pool_reserves_after_buy(
                quote_swapped_through_pool,
                base_swapped_through_pool,
            )?;

            (
                base_swapped_through_ask,
                quote_swapped_through_ask,
                base_swapped_through_pool,
                quote_swapped_through_pool,
            )
        };

        let quote_post_fee = quote_swapped_through_ask + quote_swapped_through_pool;
        let quote_in = self
            .pre_fee_adjust_rounded_down(quote_post_fee)
            .ok_or(PlasmaStateError::Overflow)?;

        let quote_fee = quote_in
            .checked_sub(quote_post_fee)
            .ok_or(PlasmaStateError::Underflow)?;

        let updated_base_reserves = self.base_reserves.upcast();
        let updated_quote_reserves = self.quote_reserves.upcast();

        let swap_result = SwapResult {
            side: Side::Buy,
            base_amount_to_transfer: base_out.downcast()?,
            quote_amount_to_transfer: quote_in.downcast()?,
            base_matched_as_limit_order: base_swapped_through_ask.downcast()?,
            quote_matched_as_limit_order: quote_swapped_through_ask.downcast()?,
            base_matched_as_swap: base_swapped_through_pool.downcast()?,
            quote_matched_as_swap: quote_swapped_through_pool.downcast()?,
            fee_in_quote: quote_fee.downcast()?,
        };

        let k_end = updated_base_reserves * updated_quote_reserves;
        if k_start > k_end {
            return Err(PlasmaStateError::InvariantViolation(k_start, k_end));
        }

        if swap_result.base_amount_to_transfer
            != swap_result.base_matched_as_limit_order + swap_result.base_matched_as_swap
        {
            return Err(PlasmaStateError::SwapAmountMismatch);
        }
        if swap_result.quote_amount_to_transfer
            != swap_result.quote_matched_as_limit_order
                + swap_result.quote_matched_as_swap
                + swap_result.fee_in_quote
        {
            return Err(PlasmaStateError::SwapAmountMismatch);
        }

        // Apply fees
        // self.apply_fees(quote_fee)?;

        Ok(swap_result)
    }
}

impl Amm {
    pub fn sell_exact_in(
        &mut self,
        slot: SlotWindow,
        base_in: u64,
    ) -> Result<SwapResult, PlasmaStateError> {
        if self.total_lp_shares == 0 {
            return Err(PlasmaStateError::UninitializedPool);
        }
        self.maybe_update_snapshot(slot);
        if base_in == 0 {
            return Ok(SwapResult::new_empty_with_side(Side::Sell));
        }
        let base_in = base_in.upcast();

        let quote_reserves = self.quote_reserves.upcast();
        let base_reserves = self.base_reserves.upcast();
        let k_start = quote_reserves
            .checked_mul(base_reserves)
            .ok_or(PlasmaStateError::Overflow)?;
        let mut quote_fee: u128 = 0;

        if base_in
            .checked_add(base_reserves)
            .ok_or(PlasmaStateError::Overflow)?
            .downcast()
            .is_err()
        {
            return Err(PlasmaStateError::SwapExactInTooLarge);
        }

        let LimitOrderConfiguration {
            size_in_base: size_on_bid_in_base,
            size_in_quote: size_on_bid_in_quote,
        } = self
            .get_limit_order_size_in_base_and_quote(Side::Sell)
            .ok_or(PlasmaStateError::Overflow)?;

        let (
            base_swapped_through_bid,
            quote_swapped_through_bid,
            base_swapped_through_pool,
            quote_swapped_through_pool,
        ) = if size_on_bid_in_base >= base_in {
            let base_swapped_through_bid = base_in;
            let mut quote_swapped_through_bid = self
                .get_complementary_limit_order_size(
                    base_swapped_through_bid,
                    Side::Sell,
                    TokenType::Base,
                )
                .ok_or(PlasmaStateError::Overflow)?;
            quote_fee = quote_fee
                .checked_add(
                    self.fee_rounded_down(quote_swapped_through_bid)
                        .ok_or(PlasmaStateError::Overflow)?,
                )
                .ok_or(PlasmaStateError::Overflow)?;
            self.update_pool_reserves_after_sell(
                base_swapped_through_bid,
                quote_swapped_through_bid,
            )?;
            quote_swapped_through_bid = quote_swapped_through_bid
                .checked_sub(quote_fee)
                .ok_or(PlasmaStateError::Underflow)?;

            let base_swapped_through_pool = 0_u128;
            let quote_swapped_through_pool = 0_u128;
            (
                base_swapped_through_bid,
                quote_swapped_through_bid,
                base_swapped_through_pool,
                quote_swapped_through_pool,
            )
        } else {
            let base_swapped_through_bid = size_on_bid_in_base;
            let mut quote_swapped_through_bid = size_on_bid_in_quote;

            quote_fee = quote_fee
                .checked_add(
                    self.fee_rounded_down(quote_swapped_through_bid)
                        .ok_or(PlasmaStateError::Overflow)?,
                )
                .ok_or(PlasmaStateError::Overflow)?;
            self.update_pool_reserves_after_sell(
                base_swapped_through_bid,
                quote_swapped_through_bid,
            )?;
            quote_swapped_through_bid = quote_swapped_through_bid
                .checked_sub(quote_fee)
                .ok_or(PlasmaStateError::Underflow)?;

            let base_swapped_through_pool = base_in
                .checked_sub(size_on_bid_in_base)
                .ok_or(PlasmaStateError::Underflow)?;
            let mut quote_swapped_through_pool =
                self.get_quote_out_from_base_in(base_swapped_through_pool);
            self.update_pool_reserves_after_sell(
                base_swapped_through_pool,
                quote_swapped_through_pool,
            )?;
            let swap_fee = self
                .fee_rounded_down(quote_swapped_through_pool)
                .ok_or(PlasmaStateError::Underflow)?;
            quote_fee = quote_fee
                .checked_add(swap_fee)
                .ok_or(PlasmaStateError::Underflow)?;
            quote_swapped_through_pool = quote_swapped_through_pool
                .checked_sub(swap_fee)
                .ok_or(PlasmaStateError::Underflow)?;

            (
                base_swapped_through_bid,
                quote_swapped_through_bid,
                base_swapped_through_pool,
                quote_swapped_through_pool,
            )
        };

        let quote_out = quote_swapped_through_bid
            .checked_add(quote_swapped_through_pool)
            .ok_or(PlasmaStateError::Overflow)?;

        let updated_base_reserves = self.base_reserves.upcast();
        let updated_quote_reserves = self.quote_reserves.upcast();

        let swap_result = SwapResult {
            side: Side::Sell,
            base_amount_to_transfer: base_in.downcast()?,
            quote_amount_to_transfer: quote_out.downcast()?,
            base_matched_as_limit_order: base_swapped_through_bid.downcast()?,
            quote_matched_as_limit_order: quote_swapped_through_bid.downcast()?,
            base_matched_as_swap: base_swapped_through_pool.downcast()?,
            quote_matched_as_swap: quote_swapped_through_pool.downcast()?,
            fee_in_quote: quote_fee.downcast()?,
        };
        let k_end = updated_base_reserves
            .checked_mul(updated_quote_reserves)
            .ok_or(PlasmaStateError::Underflow)?;
        if k_start > k_end {
            return Err(PlasmaStateError::InvariantViolation(k_start, k_end));
        }

        if swap_result.base_amount_to_transfer
            != swap_result
                .base_matched_as_limit_order
                .checked_add(swap_result.base_matched_as_swap)
                .ok_or(PlasmaStateError::Overflow)?
        {
            return Err(PlasmaStateError::SwapAmountMismatch);
        }
        if swap_result.quote_amount_to_transfer
            != swap_result
                .quote_matched_as_limit_order
                .checked_add(swap_result.quote_matched_as_swap)
                .ok_or(PlasmaStateError::Underflow)?
        {
            return Err(PlasmaStateError::SwapAmountMismatch);
        }

        // Apply fees
        // self.apply_fees(quote_fee)?;

        Ok(swap_result)
    }

    pub fn sell_exact_out(
        &mut self,
        slot: SlotWindow,
        quote_out: u64,
    ) -> Result<SwapResult, PlasmaStateError> {
        if self.total_lp_shares == 0 {
            return Err(PlasmaStateError::UninitializedPool);
        }

        self.maybe_update_snapshot(slot);

        if quote_out == 0 {
            return Ok(SwapResult::new_empty_with_side(Side::Sell));
        }

        let quote_out = quote_out.upcast();
        let quote_out_pre_fee = self
            .pre_fee_adjust_rounded_down(quote_out)
            .ok_or(PlasmaStateError::Overflow)?;
        let quote_fee = quote_out_pre_fee - quote_out;

        if self.quote_reserves < quote_out.downcast()? {
            return Err(PlasmaStateError::SwapExactOutTooLarge);
        }

        let quote_reserves = self.quote_reserves.upcast();
        let base_reserves = self.base_reserves.upcast();
        let k_start = quote_reserves * base_reserves;

        let LimitOrderConfiguration {
            size_in_base: size_on_bid_in_base,
            size_in_quote: size_on_bid_in_quote,
        } = self
            .get_limit_order_size_in_base_and_quote(Side::Sell)
            .ok_or(PlasmaStateError::Overflow)?;

        let (
            quote_swapped_through_bid,
            base_swapped_through_bid,
            quote_swapped_through_pool,
            base_swapped_through_pool,
        ) = if size_on_bid_in_quote >= quote_out_pre_fee {
            let quote_swapped_through_bid = quote_out_pre_fee;
            let base_swapped_through_bid = self
                .get_complementary_limit_order_size(
                    quote_swapped_through_bid,
                    Side::Sell,
                    TokenType::Quote,
                )
                .ok_or(PlasmaStateError::Overflow)?;

            self.update_pool_reserves_after_sell(
                base_swapped_through_bid,
                quote_swapped_through_bid,
            )?;

            let quote_swapped_through_pool = 0_u128;
            let base_swapped_through_pool = 0_u128;
            (
                quote_swapped_through_bid,
                base_swapped_through_bid,
                quote_swapped_through_pool,
                base_swapped_through_pool,
            )
        } else {
            let base_swapped_through_bid = size_on_bid_in_base;
            let quote_swapped_through_bid = size_on_bid_in_quote;

            self.update_pool_reserves_after_sell(
                base_swapped_through_bid,
                quote_swapped_through_bid,
            )?;

            let quote_swapped_through_pool = quote_out_pre_fee - quote_swapped_through_bid;
            let base_swapped_through_pool =
                self.get_base_in_from_quote_out(quote_swapped_through_pool)?;

            self.update_pool_reserves_after_sell(
                base_swapped_through_pool,
                quote_swapped_through_pool,
            )?;

            (
                quote_swapped_through_bid,
                base_swapped_through_bid,
                quote_swapped_through_pool,
                base_swapped_through_pool,
            )
        };

        let base_in: u128 = base_swapped_through_bid + base_swapped_through_pool;

        let updated_base_reserves = self.base_reserves.upcast();
        let updated_quote_reserves = self.quote_reserves.upcast();

        let swap_result = SwapResult {
            side: Side::Sell,
            base_amount_to_transfer: base_in.downcast()?,
            quote_amount_to_transfer: quote_out.downcast()?,
            base_matched_as_limit_order: base_swapped_through_bid.downcast()?,
            quote_matched_as_limit_order: quote_swapped_through_bid.downcast()?,
            base_matched_as_swap: base_swapped_through_pool.downcast()?,
            quote_matched_as_swap: quote_swapped_through_pool.downcast()?,
            fee_in_quote: quote_fee.downcast()?,
        };

        let k_end = updated_base_reserves * updated_quote_reserves;
        if k_start > k_end {
            return Err(PlasmaStateError::InvariantViolation(k_start, k_end));
        }

        if swap_result.base_amount_to_transfer
            != swap_result.base_matched_as_limit_order + swap_result.base_matched_as_swap
        {
            return Err(PlasmaStateError::SwapAmountMismatch);
        }
        if swap_result.quote_amount_to_transfer
            != swap_result.quote_matched_as_limit_order + swap_result.quote_matched_as_swap
                - swap_result.fee_in_quote
        {
            return Err(PlasmaStateError::SwapAmountMismatch);
        }

        // Apply fees
        self.apply_fees(quote_fee)?;

        Ok(swap_result)
    }
}
