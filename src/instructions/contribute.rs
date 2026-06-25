use pinocchio::{
    AccountView, ProgramResult,
    cpi::{Seed, Signer},
    error::ProgramError,
    sysvars::{Sysvar, clock::Clock, rent::Rent},
};
use pinocchio_pubkey::derive_address;
use pinocchio_system::instructions::CreateAccount;
use pinocchio_token::instructions::Transfer;

use crate::{
    constants::{MAX_CONTRIBUTION_PERCENTAGE, PERCENTAGE_SCALER, SECONDS_TO_DAYS},
    error::FundraiserError,
    state::{Contributor, Fundraiser},
};

pub fn process_contribute_instruction(accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    let [
        contributor,
        mint_to_raise,
        fundraiser,
        contributor_account,
        contributor_ata,
        vault,
        _system_program,
        _token_program,
        // _rest @ ..
    ] = accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Signer Check
    if !contributor.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Extract the payload data securely
    if data.len() < 10 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let ptr = data.as_ptr();
    let bump_contributor = unsafe { *ptr };
    let bump_fundraiser  = unsafe { *ptr.add(1) };
    let amount           = unsafe { (ptr.add(2) as *const u64).read_unaligned() };

    // Load fundraiser state (shared peek for validation, then mut borrow)
    let (maker_raw, mint_raw, amount_to_raise, current_amount, time_started, duration) = {
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
        )
    };

    // Verify fundraiser PDA
    let seeds = [b"fundraiser".as_ref(), maker_raw.as_ref(), &[bump_fundraiser]];
    let fundraiser_pda = derive_address(
        &seeds,
        None,
        &crate::ID.to_bytes(),
    );
    if fundraiser_pda != *fundraiser.address().as_array() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Verify mint_to_raise matches what fundraiser recorded
    if mint_to_raise.address().as_ref() != mint_raw.as_ref() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Derive & verify contributor PDA, init_if_needed
    let seeds = [
        b"contributor".as_ref(),
        fundraiser.address().as_array().as_ref(),
        contributor.address().as_array().as_ref(),
        &[bump_contributor],
    ];
    let contributor_pda = derive_address(
        &seeds,
        None,
        &crate::ID.to_bytes(),
    );
          
    if contributor_pda != *contributor_account.address().as_array() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Create the contributor account only if it has not been initialised yet
    // (mirrors Anchor's `init_if_needed`).
    if contributor_account.data_len() == 0 {
        let bump_bytes          = [bump_contributor];
        let fundraiser_key_bytes = fundraiser.address().as_array();
        let contributor_key_bytes = contributor.address().as_array();

        let signer_seeds = [
            Seed::from(b"contributor"),
            Seed::from(fundraiser_key_bytes.as_ref()),
            Seed::from(contributor_key_bytes.as_ref()),
            Seed::from(bump_bytes.as_ref()),
        ];
        let signer = Signer::from(&signer_seeds);

        CreateAccount {
            from: contributor,
            to: contributor_account,
            lamports: Rent::get()?.try_minimum_balance(Contributor::LEN)?,
            space: Contributor::LEN as u64,
            owner: &crate::ID,
        }
        .invoke_signed(&[signer])?;
    }

    // Load contributor state
    // We need the current accumulated amount for the cap check before we mutate.
    let contributor_accumulated = {
        let data_bytes = unsafe { contributor_account.borrow_unchecked() };
        if data_bytes.len() != Contributor::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        let c = unsafe { &*(data_bytes.as_ptr() as *const Contributor) };
        c.amount()
    };

    // Parse mint decimals for minimum-contribution check (direct read at offset 44)
    let decimals = unsafe { *mint_to_raise.borrow_unchecked().as_ptr().add(44) };

    // Business logic checks

    // 1. Contribution must be > 1 * 10^decimals (i.e. at least 1 whole token)
    let min_contribution: u64 = 10_u64
        .checked_pow(decimals as u32)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if amount <= min_contribution {
        return Err(FundraiserError::ContributionTooSmall.into());
    }

    // 2. Single contribution must not exceed MAX_CONTRIBUTION_PERCENTAGE of goal
    let max_single = amount_to_raise
        .checked_mul(MAX_CONTRIBUTION_PERCENTAGE)
        .and_then(|v| v.checked_div(PERCENTAGE_SCALER))
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if amount > max_single {
        return Err(FundraiserError::ContributionTooBig.into());
    }

    // 3. Fundraiser duration must not have elapsed yet
    let current_time = Clock::get()?.unix_timestamp;
    let elapsed_days = ((current_time - time_started) / SECONDS_TO_DAYS) as u8;
    if duration <= elapsed_days {
        return Err(FundraiserError::FundraiserEnded.into());
    }

    // 4. Cumulative contributions per contributor must stay ≤ max_single
    let new_total = contributor_accumulated
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if contributor_accumulated > max_single || new_total > max_single {
        return Err(FundraiserError::MaximumContributionsReached.into());
    }

    // CPI: transfer tokens from contributor ATA → vault
    Transfer::new(contributor_ata, vault, contributor, amount)
        .invoke()?;

    // Update fundraiser state
    {
        let fundraiser_state = Fundraiser::from_account_info(fundraiser)?;
        fundraiser_state.set_current_amount(current_amount + amount);
    }

    // Update contributor state
    {
        let contributor_state = Contributor::from_account_info(contributor_account)?;
        contributor_state.set_amount(new_total);
    }

    Ok(())
}