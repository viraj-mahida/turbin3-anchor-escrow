use anchor_lang::prelude::*;

#[error_code]
pub enum EscrowError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Mint A and mint B must be different")]
    SameMint,
    #[msg("Only the maker can do this")]
    Unauthorized,
    #[msg("Account does not belong to this escrow")]
    InvalidMint,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}
