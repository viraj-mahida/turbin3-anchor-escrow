use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Escrow {
    /// Lets one maker run several offers side by side.
    pub seed: u64,
    pub maker: Pubkey,
    /// Mint the maker deposited.
    pub mint_a: Pubkey,
    /// Mint the maker wants in return.
    pub mint_b: Pubkey,
    /// Amount of mint B the taker has to pay.
    pub receive: u64,
    pub bump: u8,
}
