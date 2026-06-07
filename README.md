# SPL Token Fundraiser — Pinocchio

A port of the [Anchor fundraiser program](https://github.com/ASCorreia/anchor-fundraiser) to [Pinocchio](https://github.com/febo/pinocchio) — a zero-dependency, low-level Solana program framework that produces extremely small, compute-efficient BPF binaries.

This example demonstrates how to create a fundraising campaign for SPL Tokens without the Anchor framework, doing all account validation, PDA derivation, and CPI calls manually.

---

## Why Pinocchio?

| | Anchor | Pinocchio |
|---|---|---|
| Discriminator overhead | 8 bytes per account | None |
| Account validation | Macro-generated | Manual (explicit checks) |
| CPI | `CpiContext::new_with_signer` | `instruction.invoke_signed(&[signer])` |
| Binary size | Larger (full framework) | Minimal |
| Compute units | Higher (reflection, borsh) | Lower (zero-copy, unsafe reads) |

Pinocchio gives you full control over every byte and compute unit at the cost of more explicit code.

---

## Architecture

### Program Entry Point

All instructions flow through a single `process_instruction` dispatcher in [`src/lib.rs`](./src/lib.rs):

```rust
pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    assert_eq!(program_id, &ID);

    let (discriminator, data) = instruction_data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match FundraiserInstructions::try_from(discriminator)? {
        FundraiserInstructions::Initialize => process_initialize_instruction(accounts, data)?,
        FundraiserInstructions::Contribute => process_contribute_instruction(accounts, data)?,
        FundraiserInstructions::Checker    => process_check_contributions_instruction(accounts, data)?,
        FundraiserInstructions::Refund     => process_refund_instruction(accounts, data)?,
    }
    Ok(())
}
```

The first byte of `instruction_data` is the **discriminator** that selects the instruction. The remaining bytes are the instruction-specific payload.

Unlike Anchor there is **no 8-byte account discriminator** written to on-chain data — the program owns the account, so ownership is the implicit discriminator.

---

## State Accounts

### Fundraiser

Defined in [`src/state/fundraiser.rs`](./src/state/fundraiser.rs):

```rust
#[repr(C)]
pub struct Fundraiser {
    pub maker:          [u8; 32],   // creator's public key
    pub mint_to_raise:  [u8; 32],   // mint the maker wants to collect
    pub amount_to_raise: [u8; 8],   // target amount (LE u64)
    pub current_amount:  [u8; 8],   // total donated so far (LE u64)
    pub time_started:    i64,       // Unix timestamp of creation
    pub duration:        u8,        // campaign length in days
    pub bump:            u8,        // canonical PDA bump
}

impl Fundraiser {
    pub const LEN: usize = 32 + 32 + 8 + 8 + 8 + 1 + 1; // 90 bytes
}
```

In this state account we store:

- **`maker`** — the public key of the person starting the fundraiser
- **`mint_to_raise`** — the SPL token mint the maker wants to collect
- **`amount_to_raise`** — the target token amount (stored as little-endian bytes)
- **`current_amount`** — running total of all contributions received
- **`time_started`** — Unix timestamp recorded at initialization (used to track elapsed days)
- **`duration`** — how many days the campaign runs for
- **`bump`** — the canonical PDA bump, stored so future instructions don't need to re-derive it

Because Pinocchio uses `#[repr(C)]` structs with a fixed layout, account data is read **zero-copy** by casting a raw pointer rather than deserializing through Borsh:

```rust
pub fn from_account_info(account_info: &mut AccountView) -> Result<&mut Self, ProgramError> {
    let data = unsafe { account_info.borrow_unchecked_mut() };
    if data.len() != Fundraiser::LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(unsafe { &mut *(data.as_mut_ptr() as *mut Self) })
}
```

### Contributor

Defined in [`src/state/contributor.rs`](./src/state/contributor.rs):

```rust
#[repr(C)]
pub struct Contributor {
    pub amount: [u8; 8],  // total tokens contributed by this wallet (LE u64)
}

impl Contributor {
    pub const LEN: usize = 8;
}
```

This account tracks how much a single contributor has deposited into a specific campaign. It is a PDA derived from the fundraiser key and the contributor's public key, ensuring one tracking account per (campaign, wallet) pair.

---

## Instructions

### 0 — `initialize`

> Creates a new fundraising campaign.

**Accounts:**

| # | Name | Writable | Signer | Description |
|---|---|---|---|---|
| 0 | `maker` | ✅ | ✅ | Campaign creator, pays for account creation |
| 1 | `mint_to_raise` | | | SPL token mint to collect |
| 2 | `fundraiser` | ✅ | | PDA account to be created (`["fundraiser", maker]`) |
| 3 | `vault_ata` | ✅ | | ATA to hold contributions (`associated_token(fundraiser, mint)`) |
| 4 | `system_program` | | | Required to create accounts |
| 5 | `token_program` | | | Required for ATA creation |
| 6 | `associated_token_program` | | | Required for ATA creation |

**Instruction data layout** (`data: &[u8]`):

| Bytes | Field | Type |
|---|---|---|
| `[0]` | `bump` | `u8` — canonical bump for the fundraiser PDA |
| `[1..9]` | `amount_to_raise` | `u64` little-endian |
| `[9]` | `duration` | `u8` — campaign length in days |

**Logic:**

```rust
// 1. Verify the maker is a signer
// 2. Parse the mint decimals
// 3. Validate: amount_to_raise > MIN_AMOUNT_TO_RAISE * 10^decimals
//    (minimum 3 whole tokens, e.g. 3_000_000 for a 6-decimal mint)
// 4. Derive & verify fundraiser PDA: ["fundraiser", maker, bump]
// 5. CreateAccount CPI → allocate 90 bytes, owned by this program
// 6. Write Fundraiser fields (maker, mint, amount, current=0, timestamp, duration, bump)
// 7. Create ATA CPI → vault owned by the fundraiser PDA
```

Key difference from Anchor: the `bump` must be **supplied by the client** in the instruction payload. Anchor derives it automatically; Pinocchio verifies it:

```rust
let fundraiser_pda = derive_address(&seeds, None, &crate::ID.to_bytes());
assert_eq!(fundraiser_pda, *fundraiser.address().as_array());
```

---

### 1 — `contribute`

> Deposit tokens into a running campaign.

**Accounts:**

| # | Name | Writable | Signer | Description |
|---|---|---|---|---|
| 0 | `contributor` | ✅ | ✅ | Person depositing tokens |
| 1 | `mint_to_raise` | | | Token mint for the campaign |
| 2 | `fundraiser` | ✅ | | The campaign's state PDA |
| 3 | `contributor_account` | ✅ | | Per-(campaign, contributor) tracking PDA; created if absent |
| 4 | `contributor_ata` | ✅ | | Contributor's token account (source) |
| 5 | `vault` | ✅ | | Campaign vault ATA (destination) |
| 6 | `system_program` | | | Required if contributor account needs to be created |
| 7 | `token_program` | | | Required for token transfer |

**Instruction data layout:**

| Bytes | Field | Type |
|---|---|---|
| `[0]` | `bump_contributor` | `u8` — bump for the contributor PDA |
| `[1]` | `bump_fundraiser` | `u8` — bump for the fundraiser PDA |
| `[2..10]` | `amount` | `u64` little-endian |

**Logic:**

```rust
// 1. Verify contributor is a signer
// 2. Snapshot fundraiser state (zero-copy unsafe read)
// 3. Verify fundraiser PDA matches derived address
// 4. Verify mint_to_raise matches fundraiser's recorded mint
// 5. Derive & verify contributor PDA: ["contributor", fundraiser_key, contributor_key, bump]
// 6. init_if_needed: if contributor_account.data_len() == 0, CreateAccount CPI
// 7. Business logic checks:
//    a. amount > 10^decimals          (at least 1 whole token)
//    b. amount <= goal * 10% / 100    (single contribution cap)
//    c. elapsed_days < duration       (campaign still running)
//    d. accumulated + amount <= cap   (cumulative cap per contributor)
// 8. Transfer CPI: contributor_ata → vault (signed by contributor)
// 9. Update fundraiser.current_amount += amount
// 10. Update contributor_account.amount += amount
```

The `init_if_needed` pattern is implemented manually — checking `data_len() == 0` instead of relying on Anchor's constraint:

```rust
if contributor_account.data_len() == 0 {
    CreateAccount { from: contributor, to: contributor_account, ... }
        .invoke_signed(&[signer])?;
}
```

---

### 2 — `checker` (check_contributions)

> Allows the maker to claim all raised tokens once the target is met. Closes the fundraiser account.

**Accounts:**

| # | Name | Writable | Signer | Description |
|---|---|---|---|---|
| 0 | `maker` | ✅ | ✅ | Campaign creator, receives tokens and rent |
| 1 | `mint_to_raise` | | | Token mint for the campaign |
| 2 | `fundraiser` | ✅ | | Campaign PDA — will be closed |
| 3 | `vault` | ✅ | | ATA holding all contributions (source) |
| 4 | `maker_ata` | ✅ | | Maker's token account (destination); created if absent |
| 5 | `token_program` | | | Required for transfer |
| 6 | `system_program` | | | Required if maker ATA needs to be created |
| 7 | `associated_token_program` | | | Required if maker ATA needs to be created |

**Instruction data layout:**

| Bytes | Field | Type |
|---|---|---|
| `[0]` | `bump` | `u8` — bump for the fundraiser PDA |

**Logic:**

```rust
// 1. Verify maker is a signer
// 2. Snapshot fundraiser state
// 3. Verify maker matches fundraiser.maker
// 4. Verify mint matches fundraiser.mint_to_raise
// 5. Verify fundraiser PDA and bump against stored bump
// 6. Verify vault is canonical ATA for (fundraiser PDA, mint)
// 7. Read vault token balance
// 8. Check: vault_amount >= amount_to_raise (target must be met)
// 9. init_if_needed: create maker_ata if data_len() == 0
// 10. Transfer CPI (PDA-signed): vault → maker_ata, full vault balance
// 11. Close fundraiser: drain lamports to maker, call fundraiser.close()
```

The fundraiser PDA signs the vault transfer on behalf of itself:

```rust
let signer_seeds = [
    Seed::from(b"fundraiser"),
    Seed::from(maker_raw.as_ref()),
    Seed::from(&[bump]),
];
Transfer::new(vault, maker_ata, fundraiser, vault_amount)
    .invoke_signed(&[Signer::from(&signer_seeds)])?;
```

Account closure in Pinocchio is done by manually zeroing the lamport balance and calling `account.close()`:

```rust
let fundraiser_lamports = fundraiser.lamports();
fundraiser.set_lamports(0);
maker.set_lamports(maker.lamports() + fundraiser_lamports);
fundraiser.close()?;
```

---

### 3 — `refund`

> Allows a contributor to reclaim their tokens after the campaign duration has elapsed without the target being met. Closes the contributor account.

**Accounts:**

| # | Name | Writable | Signer | Description |
|---|---|---|---|---|
| 0 | `contributor` | ✅ | ✅ | Person reclaiming tokens |
| 1 | `maker` | | | Campaign creator (used for PDA derivation) |
| 2 | `mint_to_raise` | | | Token mint for the campaign |
| 3 | `fundraiser` | ✅ | | Campaign's state PDA |
| 4 | `contributor_account` | ✅ | | Per-contributor tracking PDA — will be closed |
| 5 | `contributor_ata` | ✅ | | Contributor's token account (destination) |
| 6 | `vault` | ✅ | | Campaign vault ATA (source) |
| 7 | `token_program` | | | Required for transfer |
| 8 | `system_program` | | | Passed for completeness |

**Instruction data layout:**

| Bytes | Field | Type |
|---|---|---|
| `[0]` | `bump_fundraiser` | `u8` — bump for the fundraiser PDA |
| `[1]` | `bump_contributor` | `u8` — bump for the contributor PDA |

**Logic:**

```rust
// 1. Verify contributor is a signer
// 2. Snapshot fundraiser state
// 3. Verify maker, mint, fundraiser PDA, bump
// 4. Verify contributor PDA: ["contributor", fundraiser_key, contributor_key, bump]
// 5. Verify vault is canonical ATA for (fundraiser PDA, mint)
// 6. Read vault balance and contributor's deposited amount
// 7. Check: elapsed_days >= duration  (campaign must have ended)
// 8. Check: vault_amount < amount_to_raise  (target must NOT have been met)
// 9. Transfer CPI (PDA-signed): vault → contributor_ata, contributor's deposited amount
// 10. Update fundraiser.current_amount -= contributor_deposited
// 11. Close contributor_account: drain lamports to contributor, call close()
```

---

## Constants

Defined in [`src/constants.rs`](./src/constants.rs):

| Constant | Value | Purpose |
|---|---|---|
| `MIN_AMOUNT_TO_RAISE` | `3` | Minimum whole tokens a campaign must target |
| `MAX_CONTRIBUTION_PERCENTAGE` | `10` | Max % of the goal any one contributor can deposit |
| `PERCENTAGE_SCALER` | `100` | Denominator for percentage math |
| `SECONDS_TO_DAYS` | `86400` | Seconds per day for elapsed-time calculations |

---

## Error Codes

Defined in [`src/error.rs`](./src/error.rs):

| Code | Variant | Triggered when |
|---|---|---|
| 0 | `InvalidAmount` | `amount_to_raise` ≤ minimum at initialization |
| 1 | `ContributionTooSmall` | Contribution is less than 1 whole token |
| 2 | `ContributionTooBig` | Single contribution exceeds 10% of goal |
| 3 | `MaximumContributionsReached` | Cumulative contributions exceed 10% cap |
| 4 | `FundraiserEnded` | Campaign duration has elapsed (contribute) |
| 5 | `TargetNotMet` | Vault balance < goal (checker) |
| 6 | `TargetMet` | Vault balance ≥ goal (refund — can't refund if target was met) |
| 7 | `InvalidVault` | Vault address doesn't match expected ATA derivation |
| 8 | `FundraiserNotEnded` | Campaign duration hasn't elapsed yet (refund) |

---

## PDA Derivations

### Fundraiser PDA
```
seeds  = ["fundraiser", maker_pubkey]
program = this program
```

### Contributor PDA
```
seeds  = ["contributor", fundraiser_pda, contributor_pubkey]
program = this program
```

### Vault ATA
```
seeds  = [fundraiser_pda, token_program_id, mint_pubkey]
program = associated_token_program
```

The vault ATA is derived using the Associated Token Account program's canonical derivation. In `checker.rs` and `refund.rs` the vault address is verified on-chain:

```rust
let expected_vault = derive_address(
    &[fundraiser.address(), TOKEN_PROGRAM_ID, mint_to_raise.address()],
    None,
    &ATA_PROGRAM_ID.to_bytes(),
);
if expected_vault != *vault.address().as_array() {
    return Err(FundraiserError::InvalidVault.into());
}
```

---

## Key Differences from the Anchor Version

| Concern | Anchor | Pinocchio |
|---|---|---|
| Account discriminator | 8-byte prefix written by Anchor | Not used; program ownership is implicit |
| State deserialization | Borsh `#[account]` derive | Zero-copy unsafe cast via `#[repr(C)]` struct |
| PDA bump | Anchor derives canonical bump automatically | Client must supply bump; program verifies |
| Account validation | `#[derive(Accounts)]` constraints | Manual checks in instruction handler |
| `init_if_needed` | Anchor constraint | `if account.data_len() == 0` + `CreateAccount` CPI |
| Account closing | `close = target` constraint | Manual lamport drain + `account.close()` |
| Minimum amount check | `MIN_AMOUNT_TO_RAISE.pow(decimals)` | `MIN_AMOUNT_TO_RAISE * 10^decimals` (corrected) |
| Instruction routing | `#[program]` macro | First-byte discriminator + `match` |

---

## Project Structure

```
src/
├── lib.rs                  # Entry point, instruction dispatch
├── constants.rs            # Program constants
├── error.rs                # Custom error enum
├── instructions/
│   ├── mod.rs              # FundraiserInstructions enum + TryFrom
│   ├── initialize.rs       # Instruction 0: create campaign
│   ├── contribute.rs       # Instruction 1: deposit tokens
│   ├── checker.rs          # Instruction 2: claim raised tokens
│   └── refund.rs           # Instruction 3: reclaim contribution
└── state/
    ├── mod.rs
    ├── fundraiser.rs       # Fundraiser state struct
    └── contributor.rs      # Contributor state struct
```

---

## Building

```bash
cargo build-sbf
```

The compiled `.so` will be placed in `target/deploy/pinocchio_fundraiser.so`.

## Testing

Integration tests use [LiteSVM](https://github.com/LiteSVM/litesvm) for fast in-process simulation:

```bash
cargo test
```

---

## References

- [Pinocchio](https://github.com/febo/pinocchio)
- [Original Anchor fundraiser](https://github.com/ASCorreia/anchor-fundraiser)
- [LiteSVM](https://github.com/LiteSVM/litesvm)
- [SPL Token Program](https://spl.solana.com/token)
- [Associated Token Account Program](https://spl.solana.com/associated-token-account)
