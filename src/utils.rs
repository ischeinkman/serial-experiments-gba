use core::cell::Cell;
use agb::external::critical_section::{self, Mutex};

#[inline(always)]
pub const fn read_bit(value: u16, n: u8) -> bool {
    value & (1 << n) != 0
}
#[inline(always)]
pub const fn write_bit(v: u16, n: u8, bit: bool) -> u16 {
    (v & !(1 << n)) | ((bit as u16) << n)
}
#[inline(always)]
pub const fn read_bit_u8(value: u8, n: u8) -> bool {
    value & (1 << n) != 0
}
#[inline(always)]
pub const fn write_bit_u8(v: u8, n: u8, bit: bool) -> u8 {
    (v & !(1 << n)) | ((bit as u8) << n)
}

pub struct GbaCell<T> {
    inner: Mutex<Cell<T>>,
}

impl<T> GbaCell<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(Cell::new(value)),
        }
    }
    pub fn swap(&self, value: T) -> T {
        critical_section::with(|cs| self.inner.borrow(cs).replace(value))
    }
    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut().get_mut()
    }
    pub fn swap_if<F>(&self, value: T, condition: F) -> Result<T, T>
    where
        F: FnOnce(&T) -> bool,
    {
        critical_section::with(|cs| {
            let old = self.inner.borrow(cs).replace(value);
            if condition(&old) {
                Ok(old)
            } else {
                let value = self.inner.borrow(cs).replace(old);
                Err(value)
            }
        })
    }
}
