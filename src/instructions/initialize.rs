use pinocchio::{
    AccountView, ProgramResult,
    cpi::{Seed, Signer},
    error::ProgramError,
    sysvars::{Sysvar, clock::Clock, rent::Rent},
};
use pinocchio_pubkey::derive_address;
use pinocchio_system::instructions::CreateAccount;
use pinocchio_associated_token_account::instructions::Create;
use pinocchio_token::state::Mint;

use crate::state::Fundraiser;
use crate::constants::MIN_AMOUNT_TO_RAISE;
use crate::error::FundraiserError;

pub fn process_initialize_instruction(accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    let [
        maker,
        mint_to_raise,
        fundraiser,
        vault_ata,
        system_program,
        token_program,
        _associated_token_program @ ..,

    ] = accounts
    else {
       return Err(ProgramError::NotEnoughAccountKeys)
    };

    // Signer Check
    if !maker.is_signer() {
    return Err(ProgramError::MissingRequiredSignature);
    }

    // Extract the payload data securely (TODO: use unsafe later to reduce CUs)
    if data.len() < 10 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let bump = data[0];
    let amount_to_raise = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let duration = data[9];

    // Parse the mint account (can it be unsafe? can CUs be reduced?)
    let mint_state = Mint::from_account_view(mint_to_raise)?;
    let decimals = mint_state.decimals();

    // Calculate the threshold: MIN_AMOUNT_TO_RAISE * 10^decimals (e.g. 3 * 1_000_000 for 6 decimals)
    let min_required = 10_u64
        .checked_pow(decimals as u32)
        .and_then(|scale| MIN_AMOUNT_TO_RAISE.checked_mul(scale))
        .ok_or(ProgramError::ArithmeticOverflow)?;

    // Enforce the requirement
    if amount_to_raise <= min_required {
        return Err(FundraiserError::InvalidAmount.into());
    }

    // Derive PDA and verify
    let seeds = [b"fundraiser".as_ref(), maker.address().as_ref(), &[bump]];
    let fundraiser_pda = derive_address(&seeds, None, &crate::ID.to_bytes());
    assert_eq!(fundraiser_pda, *fundraiser.address().as_array());

    // Create fundraiser account
    let bump_bytes = [bump];

    let signer_seeds = [
        Seed::from(b"fundraiser"),
        Seed::from(maker.address().as_array()),
        Seed::from(bump_bytes.as_ref()),
    ];
    let signer = Signer::from(&signer_seeds);

    CreateAccount {
        from: maker,
        to: fundraiser,
        lamports: Rent::get()?.try_minimum_balance(Fundraiser::LEN)?,
        space: Fundraiser::LEN as u64,
        owner: &crate::ID,
    }
    .invoke_signed(&[signer])?;

    // Initialize fundraiser state
    let fundraiser_state = Fundraiser::from_account_info(fundraiser)?;
    
    fundraiser_state.set_maker(maker.address());
    fundraiser_state.set_mint_to_raise(mint_to_raise.address());
    fundraiser_state.set_amount_to_raise(amount_to_raise);
    fundraiser_state.set_current_amount(0);
    fundraiser_state.set_time_started(Clock::get()?.unix_timestamp);
    fundraiser_state.set_duration(duration);
    fundraiser_state.bump = bump;

    // Create vault associated token account
    Create {
        funding_account: maker,
        account: vault_ata,
        wallet: fundraiser,
        mint: mint_to_raise,
        token_program,
        system_program, 
    }
    .invoke()?;

    Ok(())
}