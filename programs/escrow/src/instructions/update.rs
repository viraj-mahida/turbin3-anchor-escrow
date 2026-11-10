use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{constants::ESCROW_SEED, error::EscrowError, state::Escrow};

#[derive(Accounts)]
pub struct Update<'info> {
    pub maker: Signer<'info>,

    #[account(mint::token_program = token_program)]
    pub mint_a: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = mint_a,
        associated_token::authority = maker,
        associated_token::token_program = token_program
    )]
    pub maker_ata_a: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        has_one = maker @ EscrowError::Unauthorized,
        has_one = mint_a @ EscrowError::InvalidMint,
        seeds = [ESCROW_SEED, escrow.maker.as_ref(), escrow.seed.to_le_bytes().as_ref()],
        bump = escrow.bump
    )]
    pub escrow: Account<'info, Escrow>,

    #[account(
        mut,
        associated_token::mint = mint_a,
        associated_token::authority = escrow,
        associated_token::token_program = token_program
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

impl<'info> Update<'info> {
    pub fn update(&mut self, new_receive: u64, new_deposit: Option<u64>) -> Result<()> {
        require!(new_receive > 0, EscrowError::InvalidAmount);
        self.escrow.receive = new_receive;

        let Some(new_deposit) = new_deposit else {
            return Ok(());
        };
        require!(new_deposit > 0, EscrowError::InvalidAmount);

        let current = self.vault.amount;
        if new_deposit > current {
            // Top up: the maker signs for the extra tokens.
            let extra = new_deposit
                .checked_sub(current)
                .ok_or(EscrowError::MathOverflow)?;
            let cpi_accounts = TransferChecked {
                from: self.maker_ata_a.to_account_info(),
                mint: self.mint_a.to_account_info(),
                to: self.vault.to_account_info(),
                authority: self.maker.to_account_info(),
            };
            let cpi_ctx = CpiContext::new(self.token_program.key(), cpi_accounts);
            transfer_checked(cpi_ctx, extra, self.mint_a.decimals)?;
        } else if new_deposit < current {
            // Shrink: the escrow PDA signs to hand the difference back.
            let excess = current
                .checked_sub(new_deposit)
                .ok_or(EscrowError::MathOverflow)?;
            let maker_key = self.maker.key();
            let seed_bytes = self.escrow.seed.to_le_bytes();
            let seeds: &[&[u8]] = &[
                ESCROW_SEED,
                maker_key.as_ref(),
                seed_bytes.as_ref(),
                &[self.escrow.bump],
            ];
            let signer_seeds = &[seeds];
            let cpi_accounts = TransferChecked {
                from: self.vault.to_account_info(),
                mint: self.mint_a.to_account_info(),
                to: self.maker_ata_a.to_account_info(),
                authority: self.escrow.to_account_info(),
            };
            let cpi_ctx =
                CpiContext::new_with_signer(self.token_program.key(), cpi_accounts, signer_seeds);
            transfer_checked(cpi_ctx, excess, self.mint_a.decimals)?;
        }

        Ok(())
    }
}
