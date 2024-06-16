use super::*;

use agb::{
    external::critical_section::CriticalSection,
    interrupt::{add_interrupt_handler, Interrupt, InterruptHandler},
};
use bulk::{BulkInitError, BulkMultiplayer};

use core::{marker::PhantomData, mem};

mod buffer;
mod bulk;
mod registers;
mod ringbuf;
use registers::MultiplayerCommReg;

/// The value used by the GBA hardware to indicate either an in-progress
/// transfer or that a slot out of the 4 available ports is currently not used
/// by a GBA.
const SENTINEL: u16 = u16::MAX;

/// The ID number of a GBA unit in the session.
#[repr(u8)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Debug, Default)]
pub enum PlayerId {
    /// Player 0, AKA the "parent" unit.
    ///
    /// This is the only unit allowed to initiate a data transfer, which will
    /// populate all 4 `SIOMULT` registers for every GBA unit in the multiplayer
    /// session.
    #[default]
    P0 = 0,
    /// Player 1
    P1 = 1,
    /// Player 2
    P2 = 2,
    /// Player 3
    P3 = 3,
}

impl PlayerId {
    /// An array of all available player IDs for easy iteration.
    pub const ALL: [PlayerId; 4] = [PlayerId::P0, PlayerId::P1, PlayerId::P2, PlayerId::P3];
}

pub struct MultiplayerSerial<'a> {
    _handle: PhantomData<&'a mut Serial>,
    buffer_interrupt: Option<InterruptHandler>,
    is_parent: bool,
    playerid: Option<PlayerId>,
    rate: BaudRate,
}

fn enter_multiplayer(rate: BaudRate) -> Result<(), MultiplayerError> {
    let rcnt = RcntWrapper::get();
    let siocnt = MultiplayerSiocnt::get();

    rcnt.set_mode(SerialMode::Multiplayer);
    siocnt.set_mode(SerialMode::Multiplayer);
    siocnt.set_baud_rate(rate);

    if siocnt.error_flag() {
        return Err(MultiplayerError::FailedOkayCheck);
    }
    Ok(())
}
/// Tells the other connected GBAs that we aren't ready to transfer yet.
///
/// This is accomplished by changing to a different Serial Mode that doesn't
/// set the SD pin to HIGH.
fn mark_unready() {
    // Joybus mode has SD low always (source: https://mgba-emu.github.io/gbatek/#sio-joy-bus-mode)
    RcntWrapper::get().set_mode(SerialMode::Joybus);
}

impl<'a> MultiplayerSerial<'a> {
    pub fn new(_handle: &'a mut Serial, rate: BaudRate) -> Result<Self, MultiplayerError> {
        let mut retvl = Self {
            _handle: PhantomData,
            buffer_interrupt: None,
            is_parent: false,
            playerid: None,
            rate,
        };
        retvl.initialize()?;
        Ok(retvl)
    }

    fn initialize(&mut self) -> Result<(), MultiplayerError> {
        enter_multiplayer(self.rate)?;
        let is_parent = MultiplayerSiocnt::get().is_parent();
        self.is_parent = is_parent;
        Ok(())
    }

    pub fn enable_bulk_mode(self, buffer_cap: usize) -> Result<BulkMultiplayer<'a>, BulkInitError> {
        BulkMultiplayer::new(self, buffer_cap)
    }
    pub fn write_send_reg(&mut self, data: u16) {
        SIOMLT_SEND.write(data)
    }
    pub fn read_player_reg_raw(&self, player: PlayerId) -> Option<u16> {
        MultiplayerCommReg::get(player).read()
    }

    /// Begins a data transfer if this is the parent unit; otherwise verifies
    /// that all a transfer can be initiated now.
    ///
    /// Does NOT block.
    pub fn start_transfer(&self) -> Result<(), TransferError> {
        let siocnt = MultiplayerSiocnt::get();
        if siocnt.busy() {
            return Err(TransferError::AlreadyInProgress);
        }
        let all_ready = self.all_ready();
        if self.is_parent {
            siocnt.start_transfer();
        }
        if !all_ready {
            return Err(TransferError::FailedReadyCheck);
        }
        if siocnt.error_flag() {
            return Err(TransferError::FailedOkayCheck);
        }
        Ok(())
    }
    pub fn enable_interrupt(&self, should_enable: bool) {
        MultiplayerSiocnt::get().enable_irq(should_enable)
    }
    pub fn interrupt_enabled(&self) -> bool {
        MultiplayerSiocnt::get().irq_enabled()
    }
    /// Adds an interrupt handler that will be triggered after a transfer
    /// finishes.
    ///
    /// # Safety
    /// The callback `cb` **must not** allocate on the heap.
    pub unsafe fn add_interrupt<F>(&mut self, cb: F)
    where
        F: Fn(CriticalSection) + Send + Sync + 'static,
    {
        self.buffer_interrupt = Some(add_interrupt_handler(Interrupt::Serial, cb));
    }
    /// Checks whether or not all other connected GBAs are ready for transfer.
    pub fn all_ready(&self) -> bool {
        MultiplayerSiocnt::get().gbas_ready()
    }

    /// Tells the other connected GBAs that we are ready for the next transfer.
    pub fn mark_ready(&mut self) {
        // Since we mark ourselves as unready by switching multiplayer modes, we
        // mark ourselves as ready just by going back into multiplayer mode
        self.initialize().ok();
    }
    /// Tells the other connected GBAs that we aren't ready to transfer yet.
    ///
    /// This is accomplished by changing to a different Serial Mode that doesn't
    /// set the SD pin to HIGH.
    pub fn mark_unready(&mut self) {
        mark_unready()
    }

    pub fn id(&self) -> Option<PlayerId> {
        self.playerid
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TransferError {
    /// Not all GBAs were ready for the transfer (though the transfer was still attempted)
    FailedReadyCheck,
    /// There was a transfer already in progress when the new one was requested.
    AlreadyInProgress,
    /// The "error" flag was tripped in the SIOCNT register.
    FailedOkayCheck,
}
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MultiplayerError {
    /// The "error" flag was tripped in the SIOCNT register.
    FailedOkayCheck,
    /// Not all GBAs were ready for the transfer (though the transfer was still attempted)
    FailedReadyCheck,
    BufferLengthMismatch, 
}

#[repr(u8)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Debug, Default)]
pub enum BaudRate {
    #[default]
    B9600 = 0,
    B38400 = 1,
    B57600 = 2,
    B115200 = 3,
}

/// Newtype extention wrapper around the Serial I/O Control register with extra
/// methods for multiplayer mode.
///
/// # GBATEK Table of Bits
/// | Bit |  Explanation       | Notes |
/// | :-- | :--                | :--   |
/// | 0-1 | Baud Rate          | (0-3: 9600,38400,57600,115200 bps)
/// | 2   | SI-Terminal        | (0=Parent, 1=Child) (Read Only)
/// | 3   | SD-Terminal        | (0=Bad connection, 1=All GBAs Ready) (Read Only)
/// | 4-5 | Multi-Player ID    | (Only valid after 1st transfer) (Read Only)
/// | 6   | Multi-Player Error | (0=Normal, 1=Error) (Read Only)
/// | 7   | Start/Busy Bit     | (0=Inactive, 1=Start/Busy) (Read Only for Slaves)
/// | 8-11| Not used           | (R/W, should be 0)
/// | 12  | Must be "0" for Multi-Player mode |
/// | 13  | Must be "1" for Multi-Player mode |
/// | 14  | IRQ Enable         | (0=Disable, 1=Want IRQ upon completion)
/// | 15  | Not used           | (Read only, always 0)
pub struct MultiplayerSiocnt {
    inner: SiocntWrapper,
}

method_wraps!(MultiplayerSiocnt, inner, SiocntWrapper);

impl MultiplayerSiocnt {
    const fn new() -> Self {
        Self {
            inner: SiocntWrapper::new(),
        }
    }
    pub const fn get() -> Self {
        Self::new()
    }
    pub fn baud_rate(&self) -> BaudRate {
        let v = self.read();
        let bits = (v & 3) as u8;
        unsafe { core::mem::transmute(bits) }
    }

    pub fn set_baud_rate(&self, rate: BaudRate) {
        let old = self.read();
        let new = (old & !3) | rate as u16;
        self.write(new)
    }

    /// Returns whether or not this unit is NOT [PlayerId::P0], aka the "parent"
    /// unit.
    ///
    /// If this is true, then this GBA unit is NOT responsible for calling
    /// [Self::start_transfer] in order to initiate each data transfer between
    /// units; instead, it must wait for the parent to transfer each word across
    /// the link. Both the parent & children can listen for completion using the
    /// Serial interrupt.
    ///
    /// # Notes
    /// Unlike [Self::id], this function can be called before any data transfers
    /// have happened yet.
    pub fn is_child(&self) -> bool {
        self.read_bit(2)
    }

    /// Returns whether or not this unit is [PlayerId::P0], aka the "parent"
    /// unit.
    ///
    /// If this is true, then this GBA unit is responsible for calling
    /// [Self::start_transfer] in order to initiate each data transfer between
    /// units.
    ///
    /// # Notes
    /// Unlike [Self::id], this function can be called before any data transfers
    /// have happened yet.
    pub fn is_parent(&self) -> bool {
        !self.is_child()
    }

    /// Checks whether or not all GBAs in the current link session are in
    /// multiplayer mode and therefore ready to receive more data.
    pub fn gbas_ready(&self) -> bool {
        self.read_bit(3)
    }

    /// Reads the current Player ID bits.
    ///
    /// # Notes
    /// This value is only valid after the first successful transfer! Before the
    /// first transfer the only ID information available is whether or not this
    /// unit is Player 0, available via [Self::is_parent] and [Self::is_child].
    pub fn id(&self) -> PlayerId {
        let regval = self.read();
        let raw = ((regval & (3 << 4)) >> 4) as u8;
        unsafe { mem::transmute(raw) }
    }

    pub fn error_flag(&self) -> bool {
        self.read_bit(6)
    }

    /// Initiates a data transfer.
    ///
    /// # Notes
    /// * This function should only be called by Player 0, AKA the "parent" GBA
    ///   unit. This can be checked using the [Self::is_parent] and
    ///   [Self::is_child] functions.
    /// * This function will immediately write the "start transfer" bit into the
    ///   register without verifying that all other GBAs are ready.
    ///
    pub fn start_transfer(&self) {
        self.write_bit(7, true)
    }

    /// Reads whether or not a transfer is currently in progress.
    pub fn busy(&self) -> bool {
        self.read_bit(7)
    }
}
