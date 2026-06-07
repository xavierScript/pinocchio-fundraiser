use pinocchio::error::ProgramError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FundraiserError {    
    /// The amount to raise does not meet the minimum requirement.
    InvalidAmount = 0, 
    
    /// The contribution is too small.
    ContributionTooSmall = 1,
    
    /// The contribution is too big.
    ContributionTooBig = 2,

    /// Maximum Contributons Reached.
    MaximumContributionsReached = 3,

    /// The fundraiser has already ended.
    FundraiserEnded = 4,

    // Target not met
    TargetNotMet = 5,

    // Target met
    TargetMet = 6,

    // Invalid Vault
    InvalidVault = 7,

    // Fundraiser not ended
    FundraiserNotEnded = 8,
}

impl From<FundraiserError> for ProgramError {
    fn from(e: FundraiserError) -> Self {
        ProgramError::Custom(e as u32)
    }
}