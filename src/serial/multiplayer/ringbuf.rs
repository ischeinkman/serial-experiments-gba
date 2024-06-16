use core::cell::Cell;
use core::{ptr, slice};

use agb::external::critical_section::{CriticalSection, Mutex};
use alloc::boxed::Box;
use alloc::vec;

use super::SENTINEL;

/// Ringbuffer for data transfers in multiplayer mode when using the "bulk
/// transfer" feature.
pub struct Ringbuffer {
    /// The head of the memory block. Should always point to an allocation of exactly `self.bufflen` elements.
    buffer: *mut u16,
    /// The maximum number of elements the buffer can store.
    bufflen: usize,
    /// The next valid location to read.
    ///
    /// Note that this value is modulus `2 * self.bufflen` instead of
    /// `self.bufflen` so we can distinguish when the buffer is "empty"
    /// (`self.read_idx == self.write_idx`) from the "full" (`self.read_idx +
    /// self.bufflen == self.write_idx`).
    read_idx: Mutex<Cell<usize>>,
    /// The next valid location to write.
    ///
    /// Note that this value is modulus `2 * self.bufflen` instead of
    /// `self.bufflen` so we can distinguish when the buffer is "empty"
    /// (`self.read_idx == self.write_idx`) from the "full" (`self.read_idx +
    /// self.bufflen == self.write_idx`).
    write_idx: Mutex<Cell<usize>>,
}

/// #SAFETY
///
/// All reads & writes to the data in this buffer are protected via critical
/// sections, meaning no matter what only 1 code path can touch it at a time.
unsafe impl Sync for Ringbuffer {}
/// #SAFETY
///
/// All reads & writes to the data in this buffer are protected via critical
/// sections, meaning no matter what only 1 code path can touch it at a time.
unsafe impl Send for Ringbuffer {}

impl Default for Ringbuffer {
    fn default() -> Self {
        Ringbuffer::empty()
    }
}
impl Drop for Ringbuffer {
    fn drop(&mut self) {
        if self.buffer.is_null() {
            return;
        }
        unsafe {
            let slice_ptr = ptr::slice_from_raw_parts_mut(self.buffer, self.bufflen);
            drop(Box::from_raw(slice_ptr));
        };
    }
}

impl Ringbuffer {
    /// Constructs an empty, nonfunctional `Ringbuffer` for use as a
    /// sentinel.
    ///
    /// Equivalent to `Ringbuffer::new(0)` but usable in a `const`
    /// context.  
    pub const fn empty() -> Self {
        Self {
            buffer: ptr::null_mut(),
            bufflen: 0,
            read_idx: Mutex::new(Cell::new(0)),
            write_idx: Mutex::new(Cell::new(0)),
        }
    }
    /// Checks whether or not this is a real `RingBuffer` or just an empty
    /// placeholder.
    pub const fn is_placeholder(&self) -> bool {
        self.bufflen == 0
    }

    /// Constructs a new ringbuffer with the given capacity.
    pub fn new(cap: usize) -> Self {
        let data = vec![SENTINEL; cap].into_boxed_slice();

        Self {
            buffer: Box::leak(data).as_mut_ptr(),
            bufflen: cap,
            read_idx: Mutex::new(Cell::new(0)),
            write_idx: Mutex::new(Cell::new(0)),
        }
    }
    pub fn push(&self, p0: u16, cs: CriticalSection) -> Result<(), ()> {
        let raw_ridx = self.read_idx.borrow(cs).get();
        let raw_widx = self.write_idx.borrow(cs).get();
        if is_full(raw_ridx, raw_widx, self.bufflen) {
            return Err(());
        }
        let widx = raw_widx % self.bufflen;
        unsafe {
            self.buffer.add(widx).write(p0);
        }
        self.write_idx
            .borrow(cs)
            .replace((raw_widx + 1) % (2 * self.bufflen));
        Ok(())
    }
    pub fn pop(&self, cs: CriticalSection) -> Option<u16> {
        let raw_ridx = self.read_idx.borrow(cs).get();
        let raw_widx = self.write_idx.borrow(cs).get();
        if is_empty(raw_ridx, raw_widx, self.bufflen) {
            return None;
        }
        let ridx = raw_ridx % self.bufflen;
        self.read_idx
            .borrow(cs)
            .replace((raw_ridx + 1) % (2 * self.bufflen));

        unsafe { Some(self.buffer.add(ridx).read()) }
    }
    /// Attempts to read multiple values from the buffer in bulk
    /// into the provided buffer.
    ///
    /// Returns the number of values read.
    pub fn read_bulk(&self, outbuff: &mut [u16], cs: CriticalSection<'_>) -> usize {
        let raw_ridx = self.read_idx.borrow(cs).get();
        let raw_widx = self.write_idx.borrow(cs).get();
        if is_empty(raw_ridx, raw_widx, self.bufflen) {
            return 0;
        }
        let mapped_ridx = raw_ridx % self.bufflen;
        let mapped_widx = raw_widx % self.bufflen;
        let buffer = unsafe { slice::from_raw_parts(self.buffer as *const _, self.bufflen) };
        let retvl = if mapped_ridx < mapped_widx {
            let to_read = (mapped_widx - mapped_ridx).min(outbuff.len());
            outbuff[..to_read].copy_from_slice(&buffer[mapped_ridx..(mapped_ridx + to_read)]);
            to_read
        } else {
            let to_read_from_first = (self.bufflen - mapped_ridx).min(outbuff.len());
            outbuff[..to_read_from_first]
                .copy_from_slice(&buffer[mapped_ridx..(mapped_ridx + to_read_from_first)]);
            if to_read_from_first < outbuff.len() {
                let to_read_from_second = (outbuff.len() - to_read_from_first).min(mapped_widx);
                outbuff[to_read_from_first..to_read_from_first + to_read_from_second]
                    .copy_from_slice(&buffer[..to_read_from_second]);
                to_read_from_first + to_read_from_second
            } else {
                to_read_from_first
            }
        };
        self.read_idx
            .borrow(cs)
            .set((raw_ridx + retvl) % (2 * self.bufflen));
        retvl
    }
    pub fn write_bulk(&self, buff: &[u16], cs: CriticalSection<'_>) -> usize {
        //TODO: Implement this
        let mut retvl = 0;
        for next in buff {
            if self.push(*next, cs).is_err() {
                return retvl;
            }
            retvl += 1;
        }
        retvl
    }
}

/// Calculates the number of elements currently stored in the ringbuffer from
/// the ringbuffer length and raw read & write indices (mod 2 * the buffer
/// length).
#[inline(always)]
const fn len(ridx: usize, widx: usize, bufflen: usize) -> usize {
    ((widx + 2 * bufflen) - ridx) % (2 * bufflen)
}

/// Checks if the ringbuffer is full based on the ringbuffer length and raw read
/// & write indices (mod 2 * the buffer length).
#[inline(always)]
const fn is_full(ridx: usize, widx: usize, bufflen: usize) -> bool {
    len(ridx, widx, bufflen) == bufflen
}

/// Checks if the ringbuffer is empty based on the ringbuffer length and raw
/// read & write indices (mod 2 * the buffer length).
#[inline(always)]
const fn is_empty(ridx: usize, widx: usize, _bufflen: usize) -> bool {
    ridx == widx
}

#[cfg(test)]
mod tests {
    use core::mem;

    use super::*;
    use agb::external::critical_section;
    use agb::Gba;

    #[test_case]
    fn verify_size(_gba: &mut Gba) {
        assert_eq!(mem::size_of::<Ringbuffer>(), 4 * mem::size_of::<usize>())
    }
    #[test_case]
    fn test_buffer_bulk(_gba: &mut Gba) {
        const BUFFER_SIZE: usize = 10;
        const OUTBUFF_SIZE: usize = 3;

        let buffer = Ringbuffer::new(BUFFER_SIZE);
        critical_section::with(|cs| {
            assert_eq!(
                buffer.write_bulk(&[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41], cs),
                BUFFER_SIZE
            );
            let mut outbuff = [0xFFFF; OUTBUFF_SIZE];
            assert_eq!(buffer.read_bulk(&mut outbuff, cs), OUTBUFF_SIZE);
            assert_eq!(outbuff, [30, 31, 32]);
            assert_eq!(buffer.read_bulk(&mut outbuff, cs), OUTBUFF_SIZE);
            assert_eq!(outbuff, [33, 34, 35]);
            assert_eq!(buffer.read_bulk(&mut outbuff, cs), OUTBUFF_SIZE);
            assert_eq!(outbuff, [36, 37, 38]);
            assert_eq!(
                buffer.read_bulk(&mut outbuff, cs),
                BUFFER_SIZE % OUTBUFF_SIZE
            );
            assert_eq!(&outbuff[..(BUFFER_SIZE % OUTBUFF_SIZE)], &[39]);
        })
    }
    #[test_case]
    fn test_buffer(_gba: &mut Gba) {
        const BUFFER_SIZE: usize = 0x8F;

        let buffer = Ringbuffer::new(BUFFER_SIZE);
        assert_eq!(buffer.bufflen, BUFFER_SIZE);

        for n in 0..(BUFFER_SIZE * 2) {
            let p0 = (n as u16) + 0x3000;
            critical_section::with(|cs| {
                let res = buffer.push(p0, cs);
                assert_eq!(res.is_err(), n >= BUFFER_SIZE, "N = {n}");
            });
        }
        critical_section::with(|cs| {
            for n in 0..BUFFER_SIZE {
                let next = buffer.pop(cs);
                assert_eq!(next, Some((n as u16) + 0x3000));
            }
            assert_eq!(buffer.pop(cs), None);
        });
    }
}
