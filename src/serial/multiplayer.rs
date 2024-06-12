use core::{marker::PhantomData, mem};

use crate::logs::println;
use agb::external::portable_atomic::{AtomicU16, Ordering};
use agb::interrupt::{add_interrupt_handler, Interrupt, InterruptHandler};

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
    pub fn enable_buffer_interrupt(&mut self) {
        self.enable_interrupt(true);
        self.buffer_interrupt = Some(unsafe { add_interrupt_handler(Interrupt::Serial, |_| {}) });
    }
    pub fn write_send_reg(&mut self, data: u16) {
        SIOMLT_SEND.write(data)
    }
    pub fn read_player_reg_raw(&self, player: PlayerId) -> Option<u16> {
        MultiplayerCommReg::new(player).read()
    }

    pub fn initialize_id(&mut self) -> Result<(), TransferError> {
        const SENTINEL: u16 = 0xFEAD;
        println!("Initializing ID");
        self.mark_unready();
        self.write_send_reg(SENTINEL);
        println!("Send register initialized");
        self.enable_interrupt(true);
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

pub struct MultiplayerCommReg {
    player_id: PlayerId,
    reg: VolAddress<u16, Safe, Safe>,
}

impl MultiplayerCommReg {
    pub const PARENT: Self = MultiplayerCommReg::new(PlayerId::Parent);
    pub const P1: Self = MultiplayerCommReg::new(PlayerId::P1);
    pub const P2: Self = MultiplayerCommReg::new(PlayerId::P2);
    pub const P3: Self = MultiplayerCommReg::new(PlayerId::P3);
    pub const ALL: [Self; 4] = [Self::PARENT, Self::P1, Self::P2, Self::P3];
    pub const fn new(player_id: PlayerId) -> Self {
        let addr = match player_id {
            PlayerId::Parent => 0x4000120,
            PlayerId::P1 => 0x4000122,
            PlayerId::P2 => 0x4000124,
            PlayerId::P3 => 0x4000126,
        };
        let reg = unsafe { VolAddress::new(addr) };
        Self { player_id, reg }
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
