use pinocchio::{AccountView, Address, error::ProgramError};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fundraiser {
    pub maker: [u8; 32],
    pub mint_to_raise: [u8; 32],
    pub amount_to_raise: [u8; 8],
    pub current_amount: [u8; 8],
    pub time_started: i64,
    pub duration: u8,
    pub bump: u8,
}


impl Fundraiser {
    pub const LEN: usize = 32 + 32 + 8 + 8 + 8 + 1 + 1;

    pub fn from_account_info(account_info: &mut AccountView) -> Result<&mut Self, ProgramError> {
        let data = unsafe {account_info.borrow_unchecked_mut()};
        if data.len() != Fundraiser::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(unsafe { &mut *(data.as_mut_ptr() as *mut Self) })
    }

    pub fn maker(&self) -> &Address {
        unsafe { &*(&self.maker as *const [u8; 32] as *const Address) }
    }

    pub fn maker_raw(&self) -> &[u8; 32] {
        &self.maker
    }

    pub fn set_maker(&mut self, maker: &Address) {
        self.maker.copy_from_slice(maker.as_ref());
    }

    pub fn mint_to_raise(&self) -> &Address {
        unsafe { &*(&self.mint_to_raise as *const [u8; 32] as *const Address) }
    }

    pub fn set_mint_to_raise(&mut self, mint: &Address) {
        self.mint_to_raise.copy_from_slice(mint.as_ref());
    }

    pub fn amount_to_raise(&self)-> u64{
        u64::from_le_bytes(self.amount_to_raise)
    }

    pub fn set_amount_to_raise(&mut self, amount:u64) {
        self.amount_to_raise = amount.to_le_bytes()
    }

    pub fn current_amount(&self) -> u64 {
        u64::from_le_bytes(self.current_amount)
    }

    pub fn set_current_amount(&mut self, amount: u64) {
        self.current_amount = amount.to_le_bytes()
    }

    pub fn time_started(&self) -> i64 {
        self.time_started
    }

    pub fn set_time_started(&mut self, time_s: i64) {
        self.time_started = time_s;
    }

    pub fn duration(&self) -> u8 {
        self.duration
    }

    pub fn set_duration(&mut self, duration: u8) {
        self.duration = duration.to_be()
    }
}