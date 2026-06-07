pub mod initialize;
pub use initialize::*;

pub mod contribute;
pub use contribute::*;

pub mod checker;
pub use checker::*;

pub mod refund;
pub use refund::*;

use pinocchio::error::ProgramError;

pub enum FundraiserInstructions {
    Initialize = 0,
    Contribute = 1,
    Checker    = 2,
    Refund     = 3,
}

impl TryFrom<&u8> for FundraiserInstructions {
    type Error = ProgramError;

    fn try_from(value: &u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(FundraiserInstructions::Initialize),
            1 => Ok(FundraiserInstructions::Contribute),
            2 => Ok(FundraiserInstructions::Checker),
            3 => Ok(FundraiserInstructions::Refund),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}