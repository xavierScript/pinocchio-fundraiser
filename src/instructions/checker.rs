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
    ID as TOKEN_PROGRAM_ID,
};
use pinocchio_associated_token_account::{
    instructions::Create,
    ID as ATA_PROGRAM_ID,
};

use crate::{
    error::FundraiserError,
    state::Fundraiser,
};

pub fn process_check_contributions_instruction(
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    let [
        maker,
        mint_to_raise,
        fundraiser,
        vault,
        maker_ata,
        token_program,
        system_program,
        associated_token_program,
    ] = accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // ── Signer check ─────────────────────────────────────────────────────────
    if !maker.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // ── Deserialize instruction data ──────────────────────────────────────────
    // Layout: [bump: u8 (1)]
    if data.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let bump = data[0];

    // ── Load fundraiser state (immutable snapshot) ────────────────────────────
    let (maker_raw, mint_raw, amount_to_raise, bump_stored) = {
        let data_bytes = unsafe { fundraiser.borrow_unchecked() };
        if data_bytes.len() != Fundraiser::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        let f = unsafe { &*(data_bytes.as_ptr() as *const Fundraiser) };
        (f.maker, f.mint_to_raise, f.amount_to_raise(), f.bump)
    };

    // ── Verify maker matches fundraiser's recorded maker ──────────────────────
    if maker.address().as_ref() != maker_raw.as_ref() {
        return Err(ProgramError::InvalidAccountData);
    }

    // ── Verify mint_to_raise matches fundraiser's recorded mint ───────────────
    if mint_to_raise.address().as_ref() != mint_raw.as_ref() {
        return Err(ProgramError::InvalidAccountData);
    }

    // ── Verify fundraiser PDA ─────────────────────────────────────────────────
    let fundraiser_pda = derive_address(
        &[b"fundraiser".as_ref(), maker_raw.as_ref(), &[bump]],
        None,
        &crate::ID.to_bytes(),
    );
    if fundraiser_pda != *fundraiser.address().as_array() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Guard against bump mismatch (caller-supplied vs stored)
    if bump != bump_stored {
        return Err(ProgramError::InvalidAccountData);
    }

    // ── Verify vault is the canonical ATA for (fundraiser PDA, mint) ─────────
    let expected_vault = derive_address(
        &[
            fundraiser.address().as_array().as_ref(),
            TOKEN_PROGRAM_ID.as_ref(),
            mint_to_raise.address().as_array().as_ref(),
        ],
        None,
        &ATA_PROGRAM_ID.to_bytes(),
    );
    if expected_vault != *vault.address().as_array() {
        return Err(FundraiserError::InvalidVault.into());
    }

    // ── Read vault token balance ──────────────────────────────────────────────
    let vault_amount = {
        let vault_state = unsafe { TokenAccount::from_account_view_unchecked(vault)? };
        vault_state.amount()
    };

    // ── Business logic: target must be met ───────────────────────────────────
    if vault_amount < amount_to_raise {
        return Err(FundraiserError::TargetNotMet.into());
    }

    // ── init_if_needed: create maker ATA if it doesn't exist yet ─────────────
    if maker_ata.data_len() == 0 {
        Create {
            funding_account: maker,
            account: maker_ata,
            wallet: maker,
            mint: mint_to_raise,
            token_program,
            system_program,
        }
        .invoke()?;
    }

    // ── Build PDA signer for the fundraiser (authority over the vault) ────────
    let bump_bytes = [bump];
    let signer_seeds = [
        Seed::from(b"fundraiser"),
        Seed::from(maker_raw.as_ref()),
        Seed::from(bump_bytes.as_ref()),
    ];
    let signer = Signer::from(&signer_seeds);

    // CPI: transfer entire vault balance → maker ATA
    Transfer::new(vault, maker_ata, fundraiser, vault_amount)
        .invoke_signed(&[signer])?;

    // ── Close the fundraiser account, returning lamports to maker ─────────────
    // Step 1: move lamports out first (runtime requires balanced lamports)
    let fundraiser_lamports = fundraiser.lamports();
    fundraiser.set_lamports(0);
    maker.set_lamports(
        maker.lamports()
            .checked_add(fundraiser_lamports)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    );

    // Step 2: zero owner + data_len (the close proper)
    fundraiser.close()?;

    Ok(())
}