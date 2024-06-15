//! Misc utility structs and functions.

use agb::external::critical_section::{self, CriticalSection, Mutex};
use core::cell::Cell;

/// Reads the `n`th bit from a `u16` as a bool. 
/// 
/// # Examples
/// ```
/// let n : u16 = (1 << 3) | (1 << 1); 
/// assert_eq!(read_bit(n, 3), true);
/// assert_eq!(read_bit(n, 2), false);
/// assert_eq!(read_bit(n, 1), true);
/// assert_eq!(read_bit(n, 0), false);
/// ```
#[inline(always)]
pub const fn read_bit(value: u16, n: u8) -> bool {
    value & (1 << n) != 0
}
#[inline(always)]
pub const fn write_bit(v: u16, n: u8, bit: bool) -> u16 {
    (v & !(1 << n)) | ((bit as u16) << n)
}
/// Reads the `n`th bit from a `u8` as a bool. 
/// 
/// # Examples
/// ```
/// let n : u8 = (1 << 3) | (1 << 1); 
/// assert_eq!(read_bit(n, 3), true);
/// assert_eq!(read_bit(n, 2), false);
/// assert_eq!(read_bit(n, 1), true);
/// assert_eq!(read_bit(n, 0), false);
/// ```
#[inline(always)]
pub const fn read_bit_u8(value: u8, n: u8) -> bool {
    value & (1 << n) != 0
}
#[inline(always)]
pub const fn write_bit_u8(v: u8, n: u8, bit: bool) -> u8 {
    (v & !(1 << n)) | ((bit as u8) << n)
}

/// An atomic [Cell] like value designed for static mutables that need to
/// communicate between interrupts and non-interrupt code.
///
/// Note that nearly all methods on this struct come in pairs: one for use
/// during interrupts which will lock a mutex and one for use during interrupts
/// which can re-use the interrupt's [CriticalSection] token. The latter are
/// marked with an `_in` at the end of their function names.
pub struct GbaCell<T> {
    inner: Mutex<Cell<T>>,
}

impl<T> GbaCell<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(Cell::new(value)),
        }
    }
    /// Atomically swaps the value in this cell while already in a [CriticalSection].
    pub fn swap(&self, value: T) -> T {
        critical_section::with(|cs| self.swap_in(cs, value))
    }
    /// Swaps the value in this cell while already in a [CriticalSection].
    /// Returns the value previously in this [GbaCell].
    pub fn swap_in(&self, cs: CriticalSection, value: T) -> T {
        self.inner.borrow(cs).replace(value)
    }
    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut().get_mut()
    }
    /// Swaps the current value with `value` only if the current value meets a
    /// condition; otherwise returns `Err(value).`
    pub fn swap_if<F>(&self, value: T, condition: F) -> Result<T, T>
    where
        F: FnOnce(&T) -> bool,
    {
        critical_section::with(|cs| self.swap_in_if(cs, value, condition))
    }
    /// Swaps the current value with `value` only if the current value meets a
    /// condition; otherwise returns `Err(value).`
    ///
    /// For use where we are already in a [CriticalSection], such as during
    /// interrupts.
    pub fn swap_in_if<F>(&self, cs: CriticalSection, value: T, condition: F) -> Result<T, T>
    where
        F: FnOnce(&T) -> bool,
    {
        let old = self.inner.borrow(cs).replace(value);
        if condition(&old) {
            Ok(old)
        } else {
            let value = self.inner.borrow(cs).replace(old);
            Err(value)
        }
    }
}

impl<T: Copy> GbaCell<T> {
    pub fn get_copy(&self) -> T {
        critical_section::with(|cs| self.get_copy_in(cs))
    }
    pub fn get_copy_in(&self, cs: CriticalSection) -> T {
        self.inner.borrow(cs).get()
    }
}

impl<T: Default> GbaCell<T> {
    pub fn lock<R>(&self, cb: impl FnOnce(&T) -> R) -> R {
        critical_section::with(|cs| self.lock_in(cs, cb))
    }
    pub fn lock_in<R>(&self, cs: CriticalSection, cb: impl FnOnce(&T) -> R) -> R {
        self.lock_mut_in(cs, |item| cb(item))
    }
    pub fn lock_mut_in<R>(&self, cs: CriticalSection, cb: impl FnOnce(&mut T) -> R) -> R {
        let sentinel = T::default();
        let mut val = self.inner.borrow(cs).replace(sentinel);
        let ret = cb(&mut val);
        self.inner.borrow(cs).set(val);
        ret
    }
    pub fn lock_mut<R>(&self, cb: impl FnOnce(&mut T) -> R) -> R {
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
