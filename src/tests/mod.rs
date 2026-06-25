#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use litesvm::LiteSVM;
    use litesvm_token::{CreateAssociatedTokenAccount, CreateMint, MintTo, spl_token};
    use solana_instruction::{AccountMeta, Instruction};
    use solana_keypair::Keypair;
    use solana_message::Message;
    use solana_native_token::LAMPORTS_PER_SOL;
    use solana_pubkey::Pubkey;
    use solana_signer::Signer;
    use solana_transaction::Transaction;

    // ─── Constants ────────────────────────────────────────────────────────────

    const TOKEN_PROGRAM_ID: Pubkey = spl_token::ID;

    /// Instruction discriminators (must match FundraiserInstructions enum order)
    const IX_INITIALIZE: u8 = 0;
    const IX_CONTRIBUTE: u8 = 1;
    const IX_CHECKER: u8 = 2;
    const IX_REFUND: u8 = 3;

    /// Fundraiser account size (must match Fundraiser::LEN)
    const FUNDRAISER_LEN: usize = 32 + 32 + 8 + 8 + 8 + 1 + 1; // 90 bytes

    /// Contributor account size (must match Contributor::LEN)
    const CONTRIBUTOR_LEN: usize = 8;

    // ─── Program / well-known addresses ──────────────────────────────────────

    fn program_id() -> Pubkey {
        Pubkey::from(crate::ID)
    }

    fn system_program() -> Pubkey {
        solana_sdk_ids::system_program::ID
    }

    fn ata_program() -> Pubkey {
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"
            .parse()
            .unwrap()
    }

    fn so_path() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // cargo build-sbf can produce either directory name depending on toolchain version
        for subdir in &["sbpf-solana-solana", "sbf-solana-solana"] {
            let p = manifest_dir
                .join("target")
                .join(subdir)
                .join("release/pinocchio_fundraiser.so");
            if p.exists() {
                return p;
            }
        }
        manifest_dir.join("target/deploy/pinocchio_fundraiser.so")
    }

    // ─── PDA helpers ─────────────────────────────────────────────────────────

    fn fundraiser_pda(maker: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"fundraiser", maker.as_ref()], &program_id())
    }

    fn contributor_pda(fundraiser: &Pubkey, contributor: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"contributor", fundraiser.as_ref(), contributor.as_ref()],
            &program_id(),
        )
    }

    fn vault_ata(fundraiser: &Pubkey, mint: &Pubkey) -> Pubkey {
        spl_associated_token_account::get_associated_token_address(fundraiser, mint)
    }

    // ─── LiteSVM setup ───────────────────────────────────────────────────────

    fn setup_svm() -> (LiteSVM, Keypair) {
        let mut svm = LiteSVM::new();
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 10 * LAMPORTS_PER_SOL)
            .expect("airdrop failed");

        let program_data = std::fs::read(so_path())
            .expect("failed to read .so — run `cargo build-sbf` first");
        svm.add_program(program_id(), &program_data)
            .expect("failed to add program");

        (svm, payer)
    }

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Once;

    static TOTAL_CUS: AtomicU64 = AtomicU64::new(0);
    static REGISTER_EXIT: Once = Once::new();

    unsafe extern "C" {
        fn atexit(cb: extern "C" fn()) -> i32;
    }

    extern "C" fn print_total_cus() {
        println!("\n==================================================");
        println!("  TOTAL COMPUTE UNITS CONSUMED ACROSS ALL TESTS: {}", TOTAL_CUS.load(Ordering::Relaxed));
        println!("==================================================\n");
    }

    /// Send a single instruction, signing with all provided keypairs.
    fn send_ix(
        svm: &mut LiteSVM,
        payer: &Keypair,
        signers: &[&Keypair],
        ix: Instruction,
    ) -> litesvm::types::TransactionMetadata {
        REGISTER_EXIT.call_once(|| {
            unsafe {
                atexit(print_total_cus);
            }
        });
        let ix_name = match ix.data.first() {
            Some(&IX_INITIALIZE) => "Initialize",
            Some(&IX_CONTRIBUTE) => "Contribute",
            Some(&IX_CHECKER) => "Checker (Check Contributions)",
            Some(&IX_REFUND) => "Refund",
            _ => "Unknown",
        };
        let msg = Message::new(&[ix], Some(&payer.pubkey()));
        let blockhash = svm.latest_blockhash();
        let tx = Transaction::new(signers, msg, blockhash);
        let meta = svm.send_transaction(tx).expect("transaction failed");
        let prev_total = TOTAL_CUS.fetch_add(meta.compute_units_consumed, Ordering::Relaxed);
        let new_total = prev_total + meta.compute_units_consumed;
        println!(">>> {} CU consumed: {} | Adding to total: {} -> New Total: {}", ix_name, meta.compute_units_consumed, meta.compute_units_consumed, new_total);
        meta
    }

    /// Like `send_ix` but returns the `Result` rather than unwrapping, so
    /// robustness tests can assert that an instruction was correctly rejected.
    fn try_send_ix(
        svm: &mut LiteSVM,
        payer: &Keypair,
        signers: &[&Keypair],
        ix: Instruction,
    ) -> Result<litesvm::types::TransactionMetadata, litesvm::types::FailedTransactionMetadata> {
        REGISTER_EXIT.call_once(|| {
            unsafe {
                atexit(print_total_cus);
            }
        });
        let ix_name = match ix.data.first() {
            Some(&IX_INITIALIZE) => "Initialize",
            Some(&IX_CONTRIBUTE) => "Contribute",
            Some(&IX_CHECKER) => "Checker (Check Contributions)",
            Some(&IX_REFUND) => "Refund",
            _ => "Unknown",
        };
        let msg = Message::new(&[ix], Some(&payer.pubkey()));
        let blockhash = svm.latest_blockhash();
        let tx = Transaction::new(signers, msg, blockhash);
        let res = svm.send_transaction(tx);
        if let Ok(ref meta) = res {
            let prev_total = TOTAL_CUS.fetch_add(meta.compute_units_consumed, Ordering::Relaxed);
            let new_total = prev_total + meta.compute_units_consumed;
            println!(">>> {} (try) CU consumed: {} | Adding to total: {} -> New Total: {}", ix_name, meta.compute_units_consumed, meta.compute_units_consumed, new_total);
        }
        res
    }

    // ─── Account data helpers ────────────────────────────────────────────────

    /// Read the SPL token balance from bytes [64..72] of a raw token account.
    fn token_balance(svm: &LiteSVM, ata: &Pubkey) -> u64 {
        let account = svm.get_account(ata).expect("token account not found");
        u64::from_le_bytes(account.data[64..72].try_into().unwrap())
    }

    /// Read the `current_amount` field out of a Fundraiser account.
    /// Layout: maker(32) | mint(32) | amount_to_raise(8) | current_amount(8) | ...
    fn fundraiser_current_amount(svm: &LiteSVM, fundraiser: &Pubkey) -> u64 {
        let account = svm.get_account(fundraiser).expect("fundraiser not found");
        u64::from_le_bytes(account.data[72..80].try_into().unwrap())
    }

    /// Read the `amount_to_raise` field out of a Fundraiser account.
    fn fundraiser_amount_to_raise(svm: &LiteSVM, fundraiser: &Pubkey) -> u64 {
        let account = svm.get_account(fundraiser).expect("fundraiser not found");
        u64::from_le_bytes(account.data[64..72].try_into().unwrap())
    }

    /// Read the `amount` field from a Contributor account (first 8 bytes).
    fn contributor_amount(svm: &LiteSVM, contributor_account: &Pubkey) -> u64 {
        let account = svm
            .get_account(contributor_account)
            .expect("contributor account not found");
        u64::from_le_bytes(account.data[0..8].try_into().unwrap())
    }

    // ─── Instruction builders ─────────────────────────────────────────────────

    /// Builds the `initialize` instruction.
    ///
    /// Instruction data layout:
    ///   [0]     discriminator = IX_INITIALIZE
    ///   [1]     bump (u8)
    ///   [2..10] amount_to_raise (u64 LE)
    ///   [10]    duration (u8)
    ///
    /// Accounts (must match process_initialize_instruction):
    ///   0. maker              — writable, signer
    ///   1. mint_to_raise      — readonly
    ///   2. fundraiser         — writable (PDA, created inside)
    ///   3. vault_ata          — writable (ATA, created inside)
    ///   4. system_program     — readonly
    ///   5. token_program      — readonly
    ///   6. associated_token_program — readonly
    fn initialize_ix(
        maker: &Pubkey,
        mint: &Pubkey,
        fundraiser: &Pubkey,
        vault: &Pubkey,
        bump: u8,
        amount_to_raise: u64,
        duration: u8,
    ) -> Instruction {
        let mut data = vec![IX_INITIALIZE, bump];
        data.extend_from_slice(&amount_to_raise.to_le_bytes());
        data.push(duration);

        Instruction {
            program_id: program_id(),
            accounts: vec![
                AccountMeta::new(*maker, true),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new(*fundraiser, false),
                AccountMeta::new(*vault, false),
                AccountMeta::new_readonly(system_program(), false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(ata_program(), false),
            ],
            data,
        }
    }

    /// Builds the `contribute` instruction.
    ///
    /// Instruction data layout:
    ///   [0]     discriminator = IX_CONTRIBUTE
    ///   [1]     bump_contributor (u8)
    ///   [2]     bump_fundraiser  (u8)
    ///   [3..11] amount (u64 LE)
    ///
    /// Accounts (must match process_contribute_instruction):
    ///   0. contributor         — writable, signer
    ///   1. mint_to_raise       — readonly
    ///   2. fundraiser          — writable
    ///   3. contributor_account — writable (PDA, created if absent)
    ///   4. contributor_ata     — writable
    ///   5. vault               — writable
    ///   6. system_program      — readonly
    ///   7. token_program       — readonly
    fn contribute_ix(
        contributor: &Pubkey,
        mint: &Pubkey,
        fundraiser: &Pubkey,
        contributor_account: &Pubkey,
        contributor_ata: &Pubkey,
        vault: &Pubkey,
        bump_contributor: u8,
        bump_fundraiser: u8,
        amount: u64,
    ) -> Instruction {
        let mut data = vec![IX_CONTRIBUTE, bump_contributor, bump_fundraiser];
        data.extend_from_slice(&amount.to_le_bytes());

        Instruction {
            program_id: program_id(),
            accounts: vec![
                AccountMeta::new(*contributor, true),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new(*fundraiser, false),
                AccountMeta::new(*contributor_account, false),
                AccountMeta::new(*contributor_ata, false),
                AccountMeta::new(*vault, false),
                AccountMeta::new_readonly(system_program(), false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data,
        }
    }

    /// Builds the `checker` (check_contributions) instruction.
    ///
    /// Instruction data layout:
    ///   [0]  discriminator = IX_CHECKER
    ///   [1]  bump (u8)
    ///
    /// Accounts (must match process_check_contributions_instruction):
    ///   0. maker                  — writable, signer
    ///   1. mint_to_raise          — readonly
    ///   2. fundraiser             — writable (will be closed)
    ///   3. vault                  — writable
    ///   4. maker_ata              — writable (created if absent)
    ///   5. token_program          — readonly
    ///   6. system_program         — readonly
    ///   7. associated_token_program — readonly
    fn checker_ix(
        maker: &Pubkey,
        mint: &Pubkey,
        fundraiser: &Pubkey,
        vault: &Pubkey,
        maker_ata: &Pubkey,
        bump: u8,
    ) -> Instruction {
        Instruction {
            program_id: program_id(),
            accounts: vec![
                AccountMeta::new(*maker, true),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new(*fundraiser, false),
                AccountMeta::new(*vault, false),
                AccountMeta::new(*maker_ata, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(system_program(), false),
                AccountMeta::new_readonly(ata_program(), false),
            ],
            data: vec![IX_CHECKER, bump],
        }
    }

    /// Builds the `refund` instruction.
    ///
    /// Instruction data layout:
    ///   [0]  discriminator = IX_REFUND
    ///   [1]  bump_fundraiser  (u8)
    ///   [2]  bump_contributor (u8)
    ///
    /// Accounts (must match process_refund_instruction):
    ///   0. contributor         — writable, signer
    ///   1. maker               — readonly
    ///   2. mint_to_raise       — readonly
    ///   3. fundraiser          — writable
    ///   4. contributor_account — writable (will be closed)
    ///   5. contributor_ata     — writable
    ///   6. vault               — writable
    ///   7. token_program       — readonly
    ///   8. system_program      — readonly
    fn refund_ix(
        contributor: &Pubkey,
        maker: &Pubkey,
        mint: &Pubkey,
        fundraiser: &Pubkey,
        contributor_account: &Pubkey,
        contributor_ata: &Pubkey,
        vault: &Pubkey,
        bump_fundraiser: u8,
        bump_contributor: u8,
    ) -> Instruction {
        Instruction {
            program_id: program_id(),
            accounts: vec![
                AccountMeta::new(*contributor, true),
                AccountMeta::new_readonly(*maker, false),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new(*fundraiser, false),
                AccountMeta::new(*contributor_account, false),
                AccountMeta::new(*contributor_ata, false),
                AccountMeta::new(*vault, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(system_program(), false),
            ],
            data: vec![IX_REFUND, bump_fundraiser, bump_contributor],
        }
    }

    // ─── Shared campaign setup ────────────────────────────────────────────────

    struct CampaignSetup {
        svm: LiteSVM,
        payer: Keypair,
        maker: Keypair,
        contributor: Keypair,
        mint: Pubkey,
        fundraiser: Pubkey,
        fundraiser_bump: u8,
        vault: Pubkey,
        contributor_ata: Pubkey,
        maker_ata: Pubkey,
        amount_to_raise: u64,
        mint_supply: u64,
    }

    /// Spins up a LiteSVM instance, deploys the program, creates a 6-decimal
    /// mint, mints `mint_supply` tokens to the contributor, and then calls
    /// `initialize` so the fundraiser + vault exist before each test runs.
    fn setup_campaign(amount_to_raise: u64, duration: u8, mint_supply: u64) -> CampaignSetup {
        let (mut svm, payer) = setup_svm();

        let maker = Keypair::new();
        let contributor = Keypair::new();

        svm.airdrop(&maker.pubkey(), 5 * LAMPORTS_PER_SOL)
            .expect("airdrop maker");
        svm.airdrop(&contributor.pubkey(), 5 * LAMPORTS_PER_SOL)
            .expect("airdrop contributor");

        // Create a 6-decimal SPL mint, authority = payer
        let mint = CreateMint::new(&mut svm, &payer)
            .decimals(6)
            .authority(&payer.pubkey())
            .send()
            .unwrap();

        // ATA for contributor (receives minted supply)
        let contributor_ata = CreateAssociatedTokenAccount::new(&mut svm, &payer, &mint)
            .owner(&contributor.pubkey())
            .send()
            .unwrap();

        // ATA for maker (receives raised tokens during check_contributions)
        let maker_ata = CreateAssociatedTokenAccount::new(&mut svm, &payer, &mint)
            .owner(&maker.pubkey())
            .send()
            .unwrap();

        // Mint initial supply to contributor
        MintTo::new(&mut svm, &payer, &mint, &contributor_ata, mint_supply)
            .send()
            .unwrap();

        // Derive PDAs
        let (fundraiser, fundraiser_bump) = fundraiser_pda(&maker.pubkey());
        let vault = vault_ata(&fundraiser, &mint);

        // Initialize the campaign
        let ix = initialize_ix(
            &maker.pubkey(),
            &mint,
            &fundraiser,
            &vault,
            fundraiser_bump,
            amount_to_raise,
            duration,
        );
        let meta = send_ix(&mut svm, &maker, &[&maker], ix);
        println!("Initialize CU: {}", meta.compute_units_consumed);

        CampaignSetup {
            svm,
            payer,
            maker,
            contributor,
            mint,
            fundraiser,
            fundraiser_bump,
            vault,
            contributor_ata,
            maker_ata,
            amount_to_raise,
            mint_supply,
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // Tests
    // ═══════════════════════════════════════════════════════════════════════════

    // ─── Initialize ──────────────────────────────────────────────────────────

    #[test]
    fn test_initialize() {
        // Use a 30-day campaign targeting 30 tokens (30_000_000 lamports for 6 dec)
        let s = setup_campaign(30_000_000, 30, 10_000_000);

        // Fundraiser account must exist and be owned by our program
        let fundraiser_account = s.svm.get_account(&s.fundraiser).expect("fundraiser not found");
        assert_eq!(
            fundraiser_account.owner,
            program_id(),
            "fundraiser must be owned by the program"
        );
        assert_eq!(
            fundraiser_account.data.len(),
            FUNDRAISER_LEN,
            "fundraiser must be exactly {} bytes",
            FUNDRAISER_LEN
        );

        // Vault ATA must exist and be owned by the token program
        let vault_account = s.svm.get_account(&s.vault).expect("vault ATA not found");
        assert_eq!(vault_account.owner, TOKEN_PROGRAM_ID, "vault must be owned by token program");

        // Vault should start empty
        assert_eq!(token_balance(&s.svm, &s.vault), 0, "vault should start empty");

        // amount_to_raise stored correctly
        assert_eq!(
            fundraiser_amount_to_raise(&s.svm, &s.fundraiser),
            s.amount_to_raise,
        );

        println!("test_initialize passed ✓");
    }

    #[test]
    fn test_initialize_rejects_amount_below_minimum() {
        let (mut svm, payer) = setup_svm();
        let maker = Keypair::new();
        svm.airdrop(&maker.pubkey(), 5 * LAMPORTS_PER_SOL).unwrap();

        let mint = CreateMint::new(&mut svm, &payer)
            .decimals(6)
            .authority(&payer.pubkey())
            .send()
            .unwrap();

        let (fundraiser, fundraiser_bump) = fundraiser_pda(&maker.pubkey());
        let vault = vault_ata(&fundraiser, &mint);

        // 2_000_000 = 2 tokens — below MIN_AMOUNT_TO_RAISE (3 tokens = 3_000_000)
        let ix = initialize_ix(
            &maker.pubkey(),
            &mint,
            &fundraiser,
            &vault,
            fundraiser_bump,
            2_000_000,
            30,
        );

        let result = try_send_ix(&mut svm, &maker, &[&maker], ix);
        assert!(result.is_err(), "should reject amount below minimum");

        println!("test_initialize_rejects_amount_below_minimum passed ✓");
    }

    // ─── Contribute ──────────────────────────────────────────────────────────

    #[test]
    fn test_contribute_once() {
        // Campaign: raise 30 tokens over 30 days
        // Mint supply: 100 tokens for the contributor
        let mut s = setup_campaign(30_000_000, 30, 100_000_000);

        let (contributor_account_pda, contributor_bump) =
            contributor_pda(&s.fundraiser, &s.contributor.pubkey());

        let contribution_amount = 1_500_000_u64; // 1.5 tokens — above the 1-token min

        let ix = contribute_ix(
            &s.contributor.pubkey(),
            &s.mint,
            &s.fundraiser,
            &contributor_account_pda,
            &s.contributor_ata,
            &s.vault,
            contributor_bump,
            s.fundraiser_bump,
            contribution_amount,
        );

        let meta = send_ix(&mut s.svm, &s.contributor, &[&s.contributor], ix);
        println!("Contribute CU: {}", meta.compute_units_consumed);

        // Vault received the contribution
        assert_eq!(
            token_balance(&s.svm, &s.vault),
            contribution_amount,
            "vault balance should equal contribution"
        );

        // Contributor ATA was debited
        assert_eq!(
            token_balance(&s.svm, &s.contributor_ata),
            s.mint_supply - contribution_amount,
            "contributor ATA should be debited"
        );

        // Contributor account tracks the amount
        assert_eq!(
            contributor_amount(&s.svm, &contributor_account_pda),
            contribution_amount,
            "contributor account should record the contribution"
        );

        // Fundraiser current_amount updated
        assert_eq!(
            fundraiser_current_amount(&s.svm, &s.fundraiser),
            contribution_amount,
        );

        println!("test_contribute_once passed ✓");
    }

    #[test]
    fn test_contribute_twice_accumulates() {
        let mut s = setup_campaign(30_000_000, 30, 100_000_000);

        let (contributor_account_pda, contributor_bump) =
            contributor_pda(&s.fundraiser, &s.contributor.pubkey());

        // First contribution
        let first_amount = 1_500_000_u64; // 1.5 tokens
        let ix1 = contribute_ix(
            &s.contributor.pubkey(),
            &s.mint,
            &s.fundraiser,
            &contributor_account_pda,
            &s.contributor_ata,
            &s.vault,
            contributor_bump,
            s.fundraiser_bump,
            first_amount,
        );
        send_ix(&mut s.svm, &s.contributor, &[&s.contributor], ix1);

        // Second contribution
        let second_amount = 1_000_000_u64; // 1 token (above min of 1 token, so > 1_000_000 needed)
        // Note: must be > 1_000_000 (strictly greater than min). Use 1_000_001.
        let second_amount = 1_000_001_u64;
        let ix2 = contribute_ix(
            &s.contributor.pubkey(),
            &s.mint,
            &s.fundraiser,
            &contributor_account_pda,
            &s.contributor_ata,
            &s.vault,
            contributor_bump,
            s.fundraiser_bump,
            second_amount,
        );
        send_ix(&mut s.svm, &s.contributor, &[&s.contributor], ix2);

        let total = first_amount + second_amount;
        assert_eq!(token_balance(&s.svm, &s.vault), total);
        assert_eq!(contributor_amount(&s.svm, &contributor_account_pda), total);
        assert_eq!(fundraiser_current_amount(&s.svm, &s.fundraiser), total);

        println!("test_contribute_twice_accumulates passed ✓");
    }

    #[test]
    fn test_contribute_rejects_too_small() {
        let mut s = setup_campaign(30_000_000, 30, 100_000_000);

        let (contributor_account_pda, contributor_bump) =
            contributor_pda(&s.fundraiser, &s.contributor.pubkey());

        // 1_000_000 == 1 token — the check is `amount <= min_contribution`
        // so exactly 1 token is rejected too
        let ix = contribute_ix(
            &s.contributor.pubkey(),
            &s.mint,
            &s.fundraiser,
            &contributor_account_pda,
            &s.contributor_ata,
            &s.vault,
            contributor_bump,
            s.fundraiser_bump,
            1_000_000,
        );
        let result = try_send_ix(&mut s.svm, &s.contributor, &[&s.contributor], ix);
        assert!(result.is_err(), "contribution of exactly 1 token should be rejected");

        println!("test_contribute_rejects_too_small passed ✓");
    }

    #[test]
    fn test_contribute_rejects_exceeding_10_percent_cap() {
        // Campaign: raise 30 tokens. 10% cap = 3_000_000 (3 tokens).
        // Contributing 4_000_000 (4 tokens) must be rejected.
        let mut s = setup_campaign(30_000_000, 30, 100_000_000);

        let (contributor_account_pda, contributor_bump) =
            contributor_pda(&s.fundraiser, &s.contributor.pubkey());

        let ix = contribute_ix(
            &s.contributor.pubkey(),
            &s.mint,
            &s.fundraiser,
            &contributor_account_pda,
            &s.contributor_ata,
            &s.vault,
            contributor_bump,
            s.fundraiser_bump,
            4_000_000, // 4 tokens > 10% of 30 tokens
        );
        let result = try_send_ix(&mut s.svm, &s.contributor, &[&s.contributor], ix);
        assert!(result.is_err(), "contribution above 10% cap should be rejected");

        println!("test_contribute_rejects_exceeding_10_percent_cap passed ✓");
    }

    #[test]
    fn test_contribute_rejects_cumulative_cap_exceeded() {
        // Campaign: 30 tokens goal → max per contributor = 3 tokens (3_000_000).
        // Contribute 2_000_000 twice → second would bring total to 4_000_000 — rejected.
        let mut s = setup_campaign(30_000_000, 30, 100_000_000);

        let (contributor_account_pda, contributor_bump) =
            contributor_pda(&s.fundraiser, &s.contributor.pubkey());

        // First contribution: 2 tokens — accepted (within cap)
        let ix1 = contribute_ix(
            &s.contributor.pubkey(),
            &s.mint,
            &s.fundraiser,
            &contributor_account_pda,
            &s.contributor_ata,
            &s.vault,
            contributor_bump,
            s.fundraiser_bump,
            2_000_000,
        );
        send_ix(&mut s.svm, &s.contributor, &[&s.contributor], ix1);

        // Second contribution: 2 tokens — cumulative total would be 4 tokens > cap
        let ix2 = contribute_ix(
            &s.contributor.pubkey(),
            &s.mint,
            &s.fundraiser,
            &contributor_account_pda,
            &s.contributor_ata,
            &s.vault,
            contributor_bump,
            s.fundraiser_bump,
            2_000_000,
        );
        let result = try_send_ix(&mut s.svm, &s.contributor, &[&s.contributor], ix2);
        assert!(result.is_err(), "cumulative contributions above 10% cap should be rejected");

        // Vault should only hold the first contribution
        assert_eq!(token_balance(&s.svm, &s.vault), 2_000_000);

        println!("test_contribute_rejects_cumulative_cap_exceeded passed ✓");
    }

    // ─── Checker (check_contributions / claim) ────────────────────────────────

    #[test]
    fn test_checker_rejects_when_target_not_met() {
        let (mut svm, payer) = setup_svm();
        let maker = Keypair::new();
        svm.airdrop(&maker.pubkey(), 5 * LAMPORTS_PER_SOL).unwrap();

        let mint = CreateMint::new(&mut svm, &payer)
            .decimals(6)
            .authority(&payer.pubkey())
            .send()
            .unwrap();

        let maker_ata = CreateAssociatedTokenAccount::new(&mut svm, &payer, &mint)
            .owner(&maker.pubkey())
            .send()
            .unwrap();

        let amount_to_raise = 10_000_000_u64;
        let (fundraiser, fundraiser_bump) = fundraiser_pda(&maker.pubkey());
        let vault = vault_ata(&fundraiser, &mint);

        send_ix(
            &mut svm, &maker, &[&maker],
            initialize_ix(&maker.pubkey(), &mint, &fundraiser, &vault, fundraiser_bump, amount_to_raise, 30),
        );

        // Mint only 5 tokens into vault — target not met
        MintTo::new(&mut svm, &payer, &mint, &vault, 5_000_000).send().unwrap();

        let ix = checker_ix(&maker.pubkey(), &mint, &fundraiser, &vault, &maker_ata, fundraiser_bump);
        let result = try_send_ix(&mut svm, &maker, &[&maker], ix);
        assert!(result.is_err(), "checker should reject when target not met");

        println!("test_checker_rejects_when_target_not_met passed ✓");
    }
}
