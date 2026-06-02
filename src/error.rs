use pinocchio::error::ProgramError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FundraiserError {    
    /// The amount to raise does not meet the minimum requirement.
    InvalidAmount = 0, 
    
    /// (Example) The fundraiser has already ended.
    FundraiserEnded = 1,
    
    /// (Example) Math overflow occurred.
    MathOverflow = 2,
}

impl From<FundraiserError> for ProgramError {
    fn from(e: FundraiserError) -> Self {
        ProgramError::Custom(e as u32)
    }
}