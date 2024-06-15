use core::cell::Cell;
use core::{ptr, slice};

use agb::external::critical_section::{CriticalSection, Mutex};
use alloc::boxed::Box;
use alloc::vec;

use super::{PlayerId, SENTINEL};

/// Ringbuffer for data transfers in multiplayer mode when using the "bulk
/// transfer" feature.
pub struct TransferBuffer {
    /// The head of the memory block. Should always point to an allocation of exactly `4 * self.bufflen` elements.
    buffer: *mut u16,
    /// The maximum number of elements the buffer can store *per player*.
    bufflen: usize,
    /// The next valid location to read for each player.
    ///
    /// Note that this value is modulus `2 * self.bufflen` instead of
    /// `self.bufflen` so we can distinguish when the buffer is "empty"
    /// (`self.read_idx == self.write_idx`) from the "full" (`self.read_idx +
    /// self.bufflen == self.write_idx`).
    read_idx: Mutex<Cell<usize>>,
    /// The next valid location to write for each player.
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
unsafe impl Sync for TransferBuffer {}
/// #SAFETY
///
/// All reads & writes to the data in this buffer are protected via critical
/// sections, meaning no matter what only 1 code path can touch it at a time.
unsafe impl Send for TransferBuffer {}

impl Default for TransferBuffer {
    fn default() -> Self {
        TransferBuffer::empty()
    }
}
impl Drop for TransferBuffer {
    fn drop(&mut self) {
        if self.buffer.is_null() {
            return;
        }
        unsafe {
            let slice_ptr = ptr::slice_from_raw_parts_mut(self.buffer, 4 * self.bufflen);
            drop(Box::from_raw(slice_ptr));
        };
    }
}

impl TransferBuffer {
    /// Constructs an empty, nonfunctional `TransferBuffer` for use as a
    /// sentinel.
    ///
    /// Equivalent to `TransferBuffer::new(&mut [])` but usable in a `const`
    /// context.  
    pub const fn empty() -> Self {
        Self {
            buffer: ptr::null_mut(),
            bufflen: 0,
            read_idx: Mutex::new(Cell::new(0)),
            write_idx: Mutex::new(Cell::new(0)),
        }
    }
    /// Checks whether or not this is a real `TransferBuffer` or just an empty
    /// placeholder.
    pub const fn is_placeholder(&self) -> bool {
        self.bufflen == 0
    }

    /// Constructs a new multiplayer bulk transfer buffer with the given capacity (per player).
    pub fn new(cap: usize) -> Self {
        let data = vec![SENTINEL; cap * 4].into_boxed_slice();

        Self {
            buffer: Box::leak(data).as_mut_ptr(),
            bufflen: cap,
            read_idx: Mutex::new(Cell::new(0)),
            write_idx: Mutex::new(Cell::new(0)),
        }
    }
    fn player_buffer_start(&self, player: PlayerId) -> *mut u16 {
        // #SAFETY
        //
        // We guarantee at creation time that `self.buffer` points to an
        // allocation that is exactly `4 * self.bufflen` long, so the resulting
        // pointer is always in bounds.

        unsafe { self.buffer.add(self.bufflen * player as usize) }
    }
    pub fn push(
        &self,
        p0: u16,
        p1: u16,
        p2: u16,
        p3: u16,
        _flags: u8,
        cs: CriticalSection,
    ) -> Result<(), ()> {
        let raw_ridx = self.read_idx.borrow(cs).get();
        let raw_widx = self.write_idx.borrow(cs).get();
        if is_full(raw_ridx, raw_widx, self.bufflen) {
            return Err(());
        }
        let widx = raw_widx % self.bufflen;
        unsafe {
            self.player_buffer_start(PlayerId::P0).add(widx).write(p0);
            self.player_buffer_start(PlayerId::P1).add(widx).write(p1);
            self.player_buffer_start(PlayerId::P2).add(widx).write(p2);
            self.player_buffer_start(PlayerId::P3).add(widx).write(p3);
        }
        self.write_idx
            .borrow(cs)
            .replace((raw_widx + 1) % (2 * self.bufflen));
        //TODO: Deal with flags
        Ok(())
    }
    pub fn pop(&self, cs: CriticalSection) -> [u16; 4] {
        let raw_ridx = self.read_idx.borrow(cs).get();
        let raw_widx = self.write_idx.borrow(cs).get();
        if is_empty(raw_ridx, raw_widx, self.bufflen) {
            return [SENTINEL; 4];
        }
        let ridx = raw_ridx % self.bufflen;
        self.read_idx
            .borrow(cs)
            .replace((raw_ridx + 1) % (2 * self.bufflen));

        unsafe {
            [
                self.player_buffer_start(PlayerId::P0).add(ridx).read(),
                self.player_buffer_start(PlayerId::P1).add(ridx).read(),
                self.player_buffer_start(PlayerId::P2).add(ridx).read(),
                self.player_buffer_start(PlayerId::P3).add(ridx).read(),
            ]
        }
    }
    /// Attempts to read multiple values from the multiplayer buffer in bulk
    /// into the provided buffers.
    ///
    /// Returns the number of values read per player into each buffer.
    ///
    /// # Notes
    /// This function may overwrite the data in `buffers` past the point where
    /// it reports having read until; as such, all data in `buffers` can be
    /// considered unspecified as soon as it is passed to this function.
    pub fn read_bulk(&self, buffers: &mut [&mut [u16]; 4], cs: CriticalSection<'_>) -> [usize; 4] {
        let ret = PlayerId::ALL.map(move |pid| {
            let buffer = &mut buffers.as_mut()[pid as usize];
            self.read_bulk_for_inner(cs, pid, buffer.as_mut())
        });
        let inc = ret.into_iter().min().unwrap_or(0);
        let prev_ridx = self.read_idx.borrow(cs).get();
        let next = (prev_ridx + inc) % (2 * self.bufflen);
        self.read_idx.borrow(cs).set(next);
        [inc; 4]
    }
    fn read_bulk_for_inner(
        &self,
        cs: CriticalSection<'_>,
        player: PlayerId,
        outbuff: &mut [u16],
    ) -> usize {
        let raw_ridx = self.read_idx.borrow(cs).get();
        let raw_widx = self.write_idx.borrow(cs).get();
        if is_empty(raw_ridx, raw_widx, self.bufflen) {
            return 0;
        }
        let mapped_ridx = raw_ridx % self.bufflen;
        let mapped_widx = raw_widx % self.bufflen;
        let buffer = self.player_buffer_start(player);
        let buffer = unsafe { slice::from_raw_parts(buffer as *const _, self.bufflen) };
        if mapped_ridx < mapped_widx {
            let to_read = (mapped_widx - mapped_ridx).min(outbuff.len());
            outbuff[..to_read].copy_from_slice(&buffer[mapped_ridx..(mapped_ridx + to_read)]);
            to_read
        } else {
            let to_read_from_first = (self.bufflen - mapped_ridx).min(outbuff.len());
            outbuff[..to_read_from_first]
                .copy_from_slice(&buffer[mapped_ridx..(mapped_ridx + to_read_from_first)]);
            if to_read_from_first >= outbuff.len() {
                return to_read_from_first;
            }
            let to_read_from_second = (outbuff.len() - to_read_from_first).min(mapped_widx);
            outbuff[to_read_from_first..to_read_from_first + to_read_from_second]
                .copy_from_slice(&buffer[..to_read_from_second]);
            to_read_from_first + to_read_from_second
        }
    }
}

/// Calculates the number of elements currently stored in the ringbuffer from
/// the ringbuffer length and raw read & write indices (mod 2 * the buffer
/// length).
#[inline(always)]
const fn len(ridx: usize, widx: usize, _bufflen: usize) -> usize {
    widx - ridx
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
        assert_eq!(
            mem::size_of::<TransferBuffer>(),
            4 * mem::size_of::<usize>()
        )
    }

    #[test_case]
    fn test_buffer(_gba: &mut Gba) {
        const BUFFER_SIZE: usize = 0x8F;
        const SENTINEL: u16 = 0xFEEF;

        let buffer = unsafe { TransferBuffer::new(BUFFER_SIZE) };
        assert_eq!(buffer.bufflen, BUFFER_SIZE);

        for n in 0..(BUFFER_SIZE * 2) {
            let (p0, p1, p2, p3, flags) = (
                (n as u16) + 0x0000,
                (n as u16) + 0x1000,
                (n as u16) + 0x2000,
                (n as u16) + 0x3000,
                0x00,
            );
            critical_section::with(|cs| {
                let res = buffer.push(p0, p1, p2, p3, flags, cs);
                assert_eq!(res.is_err(), n >= BUFFER_SIZE, "N = {n}");
            });
        }
        critical_section::with(|cs| {
            for n in 0..BUFFER_SIZE {
                let next = buffer.pop(cs);
                assert_eq!(
                    next,
                    [
                        (n as u16) + 0x0000,
                        (n as u16) + 0x1000,
                        (n as u16) + 0x2000,
                        (n as u16) + 0x3000,
                    ]
                );
            }
            assert_eq!(buffer.pop(cs), [SENTINEL; 4]);
            unsafe {
                let raw_mem = slice::from_raw_parts(buffer.buffer as *const _, buffer.bufflen * 4);
                for rawidx in 0..(BUFFER_SIZE * 4) {
                    let player = (rawidx / BUFFER_SIZE) as u16;
                    let offset = (rawidx % BUFFER_SIZE) as u16;
                    let expected = (0x1000u16 * player) + offset;
                    assert_eq!(
                        raw_mem[rawidx], expected,
                        "Error at index: {rawidx} (Player = {player}, offset = {offset})"
                    );
                }
            }
        })
    }
}
