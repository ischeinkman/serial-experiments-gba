use core::{marker::PhantomData, mem};

use crate::logs::println;
use crate::utils::GbaCell;
use agb::external::critical_section::{self, CriticalSection};
use agb::interrupt::{add_interrupt_handler, Interrupt, InterruptHandler};
use buffer::TransferBuffer;
mod buffer;
use super::*;

#[repr(u8)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Debug, Default)]
pub enum PlayerId {
    #[default]
    Parent = 0,
    P1 = 1,
    P2 = 2,
    P3 = 3,
}

impl PlayerId {
    pub const ALL: [PlayerId; 4] = [PlayerId::Parent, PlayerId::P1, PlayerId::P2, PlayerId::P3];
}

static BUFFER_SLOT: GbaCell<TransferBuffer<'static>> = GbaCell::new(TransferBuffer::PLACEHOLDER);

pub struct MultiplayerSerial<'a> {
    _handle: PhantomData<&'a mut Serial>,
    buffer_interrupt: Option<InterruptHandler>,
    is_parent: bool,
    playerid: Option<PlayerId>,
    rate: BaudRate,
}

impl<'a> MultiplayerSerial<'a> {
    pub fn new(_handle: &'a mut Serial, rate: BaudRate) -> Result<Self, InitializationError> {
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

    fn initialize(&mut self) -> Result<(), InitializationError> {
        // FROM https://rust-console.github.io/gbatek-gbaonly/#siomultiplayermode:
        let rcnt = RcntWrapper::new();
        let siocnt = MultiplayerSiocnt::get();

        rcnt.set_mode(SerialMode::Multiplayer);
        siocnt.set_mode(SerialMode::Multiplayer);
        siocnt.set_baud_rate(self.rate);

        if siocnt.error_flag() {
            return Err(InitializationError::FailedOkayCheck);
        }
        let is_parent = siocnt.is_parent();
        self.is_parent = is_parent;
        Ok(())
    }
    pub fn enable_buffer_interrupt(
        &mut self,
        buffer: &'static mut [u16],
    ) -> Result<(), InitializationError> {
        let nbuff = TransferBuffer::new(buffer);
        if BUFFER_SLOT
            .swap_if(nbuff, |old| old.is_placeholder())
            .is_err()
        {
            return Err(InitializationError::AlreadyInitialized);
        }
        self.buffer_interrupt =
            unsafe { Some(add_interrupt_handler(Interrupt::Serial, on_interrupt)) };
        self.enable_interrupt(true);
        Ok(())
    }
    pub fn disable_buffer_interrupt(&mut self) {
        self.enable_interrupt(false);
        self.buffer_interrupt = None;
        BUFFER_SLOT.swap(TransferBuffer::PLACEHOLDER);
    }
    pub fn write_send_reg(&mut self, data: u16) {
        SIOMLT_SEND.write(data)
    }
    pub fn read_player_reg_raw(&self, player: PlayerId) -> Option<u16> {
        MultiplayerCommReg::get(player).read()
    }

    pub fn is_in_bulk_mode(&self) -> bool {
        self.buffer_interrupt.is_some()
    }

    pub fn read_bulk(
        &mut self,
        buffers: &mut [&mut [u16]; 4],
    ) -> Result<[usize; 4], InitializationError> {
        if !self.is_in_bulk_mode() {
            return Err(InitializationError::NotInBulkMode);
        }
        critical_section::with(|cs| {
            let tbuf = BUFFER_SLOT.swap(TransferBuffer::PLACEHOLDER);
            Ok(tbuf.read_bulk(buffers, cs))
        })
    }

    pub fn initialize_id(&mut self) -> Result<(), TransferError> {
        const SENTINEL: u16 = 0xFEAD;
        println!("Initializing ID");
        self.mark_unready();
        self.write_send_reg(SENTINEL);
        println!("Send register initialized");
        self.mark_ready();
        loop {
            {
                println!("Performing transfer start");
                match self.start_transfer() {
                    Ok(()) => {
                        println!("Started transfer.");
                    }
                    Err(TransferError::AlreadyInProgress) => {
                        // Parent beat us to it; let it keep going
                        println!("Transfer in progress.");
                    }
                    Err(TransferError::FailedReadyCheck) => {
                        // Others are lagging; wait for them
                        println!("Failed ready check.");
                    }
                    Err(other) => {
                        return Err(other);
                    }
                };
            }

            let reg_statuses = MultiplayerCommReg::ALL.map(|reg| reg.read());
            let my_id = MultiplayerSiocnt::get().id();
            if reg_statuses[my_id as usize] == Some(SENTINEL) {
                self.playerid = Some(my_id);
                break;
            }
        }
        Ok(())
    }
    pub fn start_transfer(&self) -> Result<(), TransferError> {
        let siocnt = MultiplayerSiocnt::get();
        if siocnt.busy() {
            return Err(TransferError::AlreadyInProgress);
        }
        let all_ready = self.all_ready();
        if self.is_parent {
            println!("Doing transfer.");
            MultiplayerSiocnt::get().start_transfer();
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
        // Joybus mode has SD low always (source: https://mgba-emu.github.io/gbatek/#sio-joy-bus-mode)
        RcntWrapper::get().set_mode(SerialMode::Joybus);
    }

    pub fn id(&self) -> Option<PlayerId> {
        self.playerid
    }
}
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InitializationError {
    /// The "error" flag was tripped in the SIOCNT register.
    FailedOkayCheck,
    /// The communication memory buffer had a length not divisible by 4
    InvalidBufferLength,
    /// The serial port has already been initialized in multiplayer mode
    AlreadyInitialized,
    /// The Multiplayer handle has not been configured for bulk interrupt-based
    /// transfer
    NotInBulkMode,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TransferError {
    /// The "error" flag was tripped in the SIOCNT register.
    FailedOkayCheck,
    /// Not all GBAs were ready for the transfer (though the transfer was still attempted)
    FailedReadyCheck,
    /// There was a transfer already in progress when the new one was requested.
    AlreadyInProgress,
}

pub struct MultiplayerSiocnt {
    inner: SiocntWrapper,
}

method_wraps!(MultiplayerSiocnt, inner, SiocntWrapper);

/*
  Bit   Expl.
  0-1   Baud Rate     (0-3: 9600,38400,57600,115200 bps)
  2     SI-Terminal   (0=Parent, 1=Child)                  (Read Only)
  3     SD-Terminal   (0=Bad connection, 1=All GBAs Ready) (Read Only)
  4-5   Multi-Player ID     (0=Parent, 1-3=1st-3rd child)  (Read Only)
  6     Multi-Player Error  (0=Normal, 1=Error)            (Read Only)
  7     Start/Busy Bit      (0=Inactive, 1=Start/Busy) (Read Only for Slaves)
  8-11  Not used            (R/W, should be 0)
  12    Must be "0" for Multi-Player mode
  13    Must be "1" for Multi-Player mode
  14    IRQ Enable          (0=Disable, 1=Want IRQ upon completion)
  15    Not used            (Read only, always 0)
*/
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

    pub fn is_child(&self) -> bool {
        self.read_bit(2)
    }

    pub fn is_parent(&self) -> bool {
        !self.is_child()
    }
    pub fn gbas_ready(&self) -> bool {
        self.read_bit(3)
    }

    pub fn id(&self) -> PlayerId {
        let regval = self.read();
        let raw = ((regval & (3 << 4)) >> 4) as u8;
        unsafe { mem::transmute(raw) }
    }

    pub fn error_flag(&self) -> bool {
        self.read_bit(6)
    }

    pub fn start_transfer(&self) {
        self.write_bit(7, true)
    }
    pub fn busy(&self) -> bool {
        self.read_bit(7)
    }
}
const SIOMULTI0: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000120) };
const SIOMULTI1: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000122) };
const SIOMULTI2: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000124) };
const SIOMULTI3: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000126) };

pub struct MultiplayerCommReg {
    reg: VolAddress<u16, Safe, Safe>,
}

impl MultiplayerCommReg {
    pub const PARENT: Self = MultiplayerCommReg::new(PlayerId::Parent);
    pub const P1: Self = MultiplayerCommReg::new(PlayerId::P1);
    pub const P2: Self = MultiplayerCommReg::new(PlayerId::P2);
    pub const P3: Self = MultiplayerCommReg::new(PlayerId::P3);
    pub const ALL: [Self; 4] = [Self::PARENT, Self::P1, Self::P2, Self::P3];
    const fn new(player_id: PlayerId) -> Self {
        let reg = match player_id {
            PlayerId::Parent => SIOMULTI0,
            PlayerId::P1 => SIOMULTI1,
            PlayerId::P2 => SIOMULTI2,
            PlayerId::P3 => SIOMULTI3,
        };
        Self { reg }
    }
    pub const fn get(player_id: PlayerId) -> &'static Self {
        &Self::ALL[player_id as usize]
    }

    pub fn read(&self) -> Option<u16> {
        let raw = self.raw_read();
        if raw == 0xFFFF {
            None
        } else {
            Some(raw)
        }
    }
    pub fn raw_read(&self) -> u16 {
        self.reg.read()
    }
    pub fn is_transfering(&self) -> bool {
        self.raw_read() == 0xFFFF
    }
}

fn on_interrupt(cs: CriticalSection<'_>) {
    let siocnt = MultiplayerSiocnt::get();
    let flags = (siocnt.read() & 0xFF) as u8;
    let p0 = MultiplayerCommReg::get(PlayerId::Parent).raw_read();
    let p1 = MultiplayerCommReg::get(PlayerId::P1).raw_read();
    let p2 = MultiplayerCommReg::get(PlayerId::P2).raw_read();
    let p3 = MultiplayerCommReg::get(PlayerId::P3).raw_read();

    // We're already in a critical section, so this won't break anything.
    let tbuff = BUFFER_SLOT.swap(TransferBuffer::PLACEHOLDER);
    let _res = tbuff.push(p0, p1, p2, p3, flags, cs);
    //TODO: handle error
}
