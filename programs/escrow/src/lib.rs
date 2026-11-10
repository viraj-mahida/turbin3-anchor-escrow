pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("Du6GA4zfpnQByPxcC1DPZ1tHzBPrQyMpEn7Ehv6HHwud");

#[program]
pub mod escrow {
    use super::*;

    /// Maker locks `deposit` of mint A and asks for `receive` of mint B.
    pub fn make(ctx: Context<Make>, seed: u64, deposit: u64, receive: u64) -> Result<()> {
        ctx.accounts.init_escrow(seed, receive, &ctx.bumps)?;
        ctx.accounts.deposit(deposit)
    }

    /// Taker pays the maker in mint B and receives everything in the vault.
    pub fn take(ctx: Context<Take>) -> Result<()> {
        ctx.accounts.pay_maker()?;
        ctx.accounts.withdraw_and_close_vault()
    }

    /// Maker cancels the offer and gets mint A back.
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        ctx.accounts.refund_and_close_vault()
    }

    /// Maker changes the price, and optionally resizes the deposit.
    pub fn update(ctx: Context<Update>, new_receive: u64, new_deposit: Option<u64>) -> Result<()> {
        ctx.accounts.update(new_receive, new_deposit)
    }
}
