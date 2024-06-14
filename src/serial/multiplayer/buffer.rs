use core::borrow::BorrowMut;
use core::cell::Cell;
use core::marker::PhantomData;
use core::{ptr, slice};

use agb::external::critical_section::{self, CriticalSection, Mutex};
use agb::external::portable_atomic::{AtomicUsize, Ordering};

use super::PlayerId;

pub struct TransferBuffer<'a> {
    p0_buffer: *mut u16,
    p1_buffer: *mut u16,
    p2_buffer: *mut u16,
    p3_buffer: *mut u16,
    bufflen: usize,
    read_idx: Mutex<Cell<usize>>,
    write_idx: Mutex<Cell<usize>>,
    _phantom: PhantomData<&'a mut [u16]>,
}

/// #SAFETY
///
/// All reads & writes to the data in this buffer are protected via critical
/// sections, meaning no matter what only 1 code path can touch it at a time.
unsafe impl<'a> Sync for TransferBuffer<'a> {}
/// #SAFETY
///
/// All reads & writes to the data in this buffer are protected via critical
/// sections, meaning no matter what only 1 code path can touch it at a time.
unsafe impl<'a> Send for TransferBuffer<'a> {}

impl<'a> TransferBuffer<'a> {
    pub const PLACEHOLDER: Self = Self {
        p0_buffer: ptr::null_mut(),
        p1_buffer: ptr::null_mut(),
        p2_buffer: ptr::null_mut(),
        p3_buffer: ptr::null_mut(),
        bufflen: 0,
        read_idx: Mutex::new(Cell::new(0)),
        write_idx: Mutex::new(Cell::new(0)),
        _phantom: PhantomData,
    };
    pub const fn is_placeholder(&self) -> bool {
        self.bufflen == 0
    }
    pub fn new(memory: &'a mut [u16]) -> Self {
        debug_assert!(memory.len() % 4 == 0);
        let bufflen = memory.len() / 4;
        let (left, right) = memory.split_at_mut(memory.len() / 2);
        let (p0_buffer, p1_buffer) = left.split_at_mut(bufflen);
        let (p2_buffer, p3_buffer) = right.split_at_mut(bufflen);
        p0_buffer.fill(0xFFFF);
        p1_buffer.fill(0xFFFF);
        p2_buffer.fill(0xFFFF);
        p3_buffer.fill(0xFFFF);
        Self {
            p0_buffer: p0_buffer.as_mut_ptr(),
            p1_buffer: p1_buffer.as_mut_ptr(),
            p2_buffer: p2_buffer.as_mut_ptr(),
            p3_buffer: p3_buffer.as_mut_ptr(),
            bufflen,
            read_idx: Mutex::new(Cell::new(0)),
            write_idx: Mutex::new(Cell::new(0)),
            _phantom: PhantomData,
        }
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
            self.p0_buffer.add(widx).write(p0);
            self.p1_buffer.add(widx).write(p1);
            self.p2_buffer.add(widx).write(p2);
            self.p3_buffer.add(widx).write(p3);
        }
        self.write_idx
            .borrow(cs)
            .replace((raw_widx + 1) % (2 * self.bufflen));
        //TODO: Deal with flags
        Ok(())
    }
    pub fn pop(&self) -> [u16; 4] {
        critical_section::with(move |cs| {
            let raw_ridx = self.read_idx.borrow(cs).get();
            let raw_widx = self.write_idx.borrow(cs).get();
            if is_empty(raw_ridx, raw_widx, self.bufflen) {
                return [u16::MAX; 4];
            }
            let ridx = raw_ridx % self.bufflen;
            let retvl = unsafe {
                [
                    self.p0_buffer.add(ridx).read(),
                    self.p1_buffer.add(ridx).read(),
                    self.p2_buffer.add(ridx).read(),
                    self.p3_buffer.add(ridx).read(),
                ]
            };
            self.read_idx
                .borrow(cs)
                .replace((raw_ridx + 1) % (2 * self.bufflen));
            retvl
        })
    }
    pub fn read_bulk_for_player(&self, player: PlayerId, buffer: &mut [u16]) -> usize {
        critical_section::with(|cs| self.read_bulk_for_inner(cs, player, buffer))
    }
    pub fn read_bulk(
        &self,
        mut buffers: &mut [&mut [u16]; 4],
        cs: CriticalSection<'_>,
    ) -> [usize; 4] {
        PlayerId::ALL.map(move |pid| {
            let buffer = &mut buffers.as_mut()[pid as usize];
            self.read_bulk_for_inner(cs, pid, buffer.as_mut())
        })
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
        let buffer = match player {
            PlayerId::Parent => self.p0_buffer,
            PlayerId::P1 => self.p1_buffer,
            PlayerId::P2 => self.p2_buffer,
            PlayerId::P3 => self.p3_buffer,
        };
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

const fn len(ridx: usize, widx: usize, bufflen: usize) -> usize {
    if bufflen == 0 {
        0
    } else {
        (widx - ridx) % (2 * bufflen)
    }
}
const fn is_full(ridx: usize, widx: usize, bufflen: usize) -> bool {
    len(ridx, widx, bufflen) == bufflen
}
const fn is_empty(ridx: usize, widx: usize, _bufflen: usize) -> bool {
    ridx == widx
}

#[cfg(test)]
mod tests {
    use super::*;
    use agb::Gba;

    #[test_case]
    fn test_buffer(_gba: &mut Gba) {
        const BUFFER_SIZE: usize = 0x8F;
        const PADDING: usize = 60;
        const SENTINEL: u16 = 0xFEEF;

        const EMEM_SIZE: usize = BUFFER_SIZE * 4;
        const FULL_MEM_SIZE: usize = EMEM_SIZE + PADDING;
        static mut raw_mem: [u16; FULL_MEM_SIZE] = [SENTINEL; FULL_MEM_SIZE];
        let buffer = unsafe { TransferBuffer::new(&mut raw_mem[..EMEM_SIZE]) };
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
        for n in 0..BUFFER_SIZE {
            let next = buffer.pop();
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
        assert_eq!(buffer.pop(), [0xFFFF; 4]);
        unsafe {
            for rawidx in 0..FULL_MEM_SIZE {
                if rawidx >= EMEM_SIZE {
                    assert_eq!(
                        raw_mem[rawidx], SENTINEL,
                        "Error at index: {rawidx} (PADDING)"
                    );
                    continue;
                }
                let player = (rawidx / BUFFER_SIZE) as u16;
                let offset = (rawidx % BUFFER_SIZE) as u16;
                let expected = (0x1000u16 * player) + offset;
                assert_eq!(
                    raw_mem[rawidx], expected,
                    "Error at index: {rawidx} (Player = {player}, offset = {offset})"
                );
            }
        }
    }
}
