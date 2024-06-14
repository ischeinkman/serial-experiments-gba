use agb::external::critical_section::{self, CriticalSection, Mutex};
use core::cell::Cell;

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

impl<T: Copy> GbaCell<T> {
    pub fn get_copy(&self) -> T {
        critical_section::with(|cs| self.inner.borrow(cs).get())
    }
}

impl<T: Default> GbaCell<T> {
    pub fn lock<F, R>(&self, cb: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        critical_section::with(|cs| self.lock_in(cs, cb))
    }
    pub fn lock_in<F, R>(&self, cs: CriticalSection, cb: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let sentinel = T::default();
        let val = self.inner.borrow(cs).replace(sentinel);
        let ret = cb(&val);
        self.inner.borrow(cs).set(val);
        ret
    }
    pub fn lock_mut_in<F, R>(&self, cs: CriticalSection, cb: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let sentinel = T::default();
        let mut val = self.inner.borrow(cs).replace(sentinel);
        let ret = cb(&mut val);
        self.inner.borrow(cs).set(val);
        ret
    }
    pub fn lock_mut<F, R>(&self, cb: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        critical_section::with(|cs| self.lock_mut_in(cs, cb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agb::Gba;
    #[test_case]
    fn test_bitfuncs(_gba: &mut Gba) {
        for n in 0..16 {
            let set = write_bit(0, n, true);
            assert_eq!(1 << n, set);
            assert_eq!(read_bit(set, n), true);
            assert_eq!(write_bit(set, n, false), 0);
        }
    }
    #[test_case]
    fn test_bitfuncs_u8(_gba: &mut Gba) {
        for n in 0..8 {
            let set = write_bit_u8(0, n, true);
            assert_eq!(1 << n, set);
            assert_eq!(read_bit_u8(set, n), true);
            assert_eq!(write_bit_u8(set, n, false), 0);
        }
    }
}
