use pinocchio::{
    AccountView, ProgramResult,
    cpi::{Seed, Signer},
    error::ProgramError,
    sysvars::{Sysvar, clock::Clock},
};
use pinocchio_pubkey::derive_address;
use pinocchio_token::{
    instructions::Transfer,
    state::Account as TokenAccount,
};

use crate::{
    constants::SECONDS_TO_DAYS,
    error::FundraiserError,
    state::{Contributor, Fundraiser},
};

pub fn process_refund_instruction(
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    let [
        contributor,
        maker,
        mint_to_raise,
        fundraiser,
        contributor_account,
        contributor_ata,
        vault,
        _token_program,
        _system_program,
    ] = accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Signer check
    if !contributor.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Deserialize instruction data
    // Layout: [bump_fundraiser: u8 (1), bump_contributor: u8 (1)]
    if data.len() < 2 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let ptr = data.as_ptr();
    let bump_fundraiser   = unsafe { *ptr };
    let bump_contributor  = unsafe { *ptr.add(1) };

    // Load fundraiser state (immutable snapshot)
    let (maker_raw, mint_raw, amount_to_raise, current_amount, time_started, duration, bump_stored) = {
        let data_bytes = unsafe { fundraiser.borrow_unchecked() };
        if data_bytes.len() != Fundraiser::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        let f = unsafe { &*(data_bytes.as_ptr() as *const Fundraiser) };
        (
            f.maker,
            f.mint_to_raise,
            f.amount_to_raise(),
            f.current_amount(),
            f.time_started(),
            f.duration(),
            f.bump,
        )
    };

    // Verify maker matches fundraiser's recorded maker
    if maker.address().as_ref() != maker_raw.as_ref() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Verify mint_to_raise matches fundraiser's recorded mint
    if mint_to_raise.address().as_ref() != mint_raw.as_ref() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Verify fundraiser PDA
    let fundraiser_pda = derive_address(
        &[b"fundraiser".as_ref(), maker_raw.as_ref(), &[bump_fundraiser]],
        None,
        &crate::ID.to_bytes(),
    );
    if fundraiser_pda != *fundraiser.address().as_array() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Guard against bump mismatch (caller-supplied vs stored)
    if bump_fundraiser != bump_stored {
        return Err(ProgramError::InvalidAccountData);
    }

    // Verify contributor PDA
    let contributor_pda = derive_address(
        &[
            b"contributor".as_ref(),
            fundraiser.address().as_array().as_ref(),
            contributor.address().as_array().as_ref(),
            &[bump_contributor],
        ],
        None,
        &crate::ID.to_bytes(),
    );
    if contributor_pda != *contributor_account.address().as_array() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Verify vault is canonical (owner is fundraiser PDA, mint matches mint_to_raise)
    let vault_state = unsafe { TokenAccount::from_account_view_unchecked(vault)? };
    if vault_state.owner() != fundraiser.address() {
        return Err(FundraiserError::InvalidVault.into());
    }
    if vault_state.mint() != mint_to_raise.address() {
        return Err(FundraiserError::InvalidVault.into());
    }
    let vault_amount = vault_state.amount();

    let contributor_deposited = {
        let data_bytes = unsafe { contributor_account.borrow_unchecked() };
        if data_bytes.len() != Contributor::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        let c = unsafe { &*(data_bytes.as_ptr() as *const Contributor) };
        c.amount()
    };

    // ── Business logic checks ─────────────────────────────────────────────────

    // 1. Fundraiser duration must have elapsed (refund only available after end)
    let current_time  = Clock::get()?.unix_timestamp;
    let elapsed_days  = ((current_time - time_started) / SECONDS_TO_DAYS) as u8;
    if elapsed_days < duration {
        return Err(FundraiserError::FundraiserNotEnded.into());
    }

    // 2. Target must NOT have been met (if target was met, use check_contributions)
    if vault_amount >= amount_to_raise {
        return Err(FundraiserError::TargetMet.into());
    }

    // Build PDA signer for fundraiser (vault authority)
    let bump_bytes   = [bump_fundraiser];
    let signer_seeds = [
        Seed::from(b"fundraiser"),
        Seed::from(maker_raw.as_ref()),
        Seed::from(bump_bytes.as_ref()),
    ];
    let signer = Signer::from(&signer_seeds);

    // CPI: transfer contributor's tokens vault → contributor ATA
    Transfer::new(vault, contributor_ata, fundraiser, contributor_deposited)
        .invoke_signed(&[signer])?;

    // Update fundraiser: reduce current_amount by what was refunded
    {
        let fundraiser_state = Fundraiser::from_account_info(fundraiser)?;
        fundraiser_state.set_current_amount(
            current_amount
                .checked_sub(contributor_deposited)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        );
    }

    // Close contributor account, returning lamports to contributor
    let contributor_account_lamports = contributor_account.lamports();
    contributor_account.set_lamports(0);
    contributor.set_lamports(
        contributor.lamports()
            .checked_add(contributor_account_lamports)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    );
    contributor_account.close()?;

    Ok(())
}