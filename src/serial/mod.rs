use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use voladdress::{Safe, VolAddress};

use crate::utils::{read_bit, write_bit};

pub struct Serial {
    _phanton: PhantomData<()>,
}
impl Serial {
    pub fn new() -> Self {
        Self {
            _phanton: PhantomData,
        }
    }
}

const RCNT: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000134) };
const SIOCNT: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000128) };
const SIOMLT_SEND: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x400012A) };
const SIOMULTI0: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000120) };
const SIOMULTI1: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000122) };
const SIOMULTI2: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000124) };
const SIOMULTI3: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000126) };

#[derive(PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Clone, Copy)]
pub enum Pin {
    SC = 0,
    SD = 1,
    SI = 2,
    SO = 3,
}

pub struct RegisterWrapper {
    addr: VolAddress<u16, Safe, Safe>,
}

impl RegisterWrapper {
    pub const fn new(addr: VolAddress<u16, Safe, Safe>) -> Self {
        Self { addr }
    }
    pub fn read(&self) -> u16 {
        self.addr.read()
    }
    pub fn write(&self, n: u16) {
        self.addr.write(n)
    }
    pub fn read_bit(&self, n: u8) -> bool {
        read_bit(self.addr.read(), n)
    }
    pub fn write_bit(&self, n: u8, value: bool) {
        self.addr.write(write_bit(self.addr.read(), n, value));
    }
}
macro_rules! method_wraps {
    ($child:ty, $field:ident, $parent:ty) => {
        impl AsRef<$parent> for $child {
            fn as_ref(&self) -> &$parent {
                &self.$field
            }
        }
        impl Deref for $child {
            type Target = $parent;
            fn deref(&self) -> &Self::Target {
                self.as_ref()
            }
        }
        impl AsMut<$parent> for $child {
            fn as_mut(&mut self) -> &mut $parent {
                &mut self.$field
            }
        }
        impl DerefMut for $child {
            fn deref_mut(&mut self) -> &mut Self::Target {
                self.as_mut()
            }
        }
    };
}

pub struct RcntWrapper {
    reg: RegisterWrapper,
}
method_wraps!(RcntWrapper, reg, RegisterWrapper);

impl RcntWrapper {
    pub const fn new() -> Self {
        Self {
            reg: RegisterWrapper::new(RCNT),
        }
    }
    pub const fn get() -> Self {
        Self::new()
    }
    pub fn sc_data(&self) -> bool {
        self.reg.read_bit(0)
    }
    pub fn write_sc_data(&self, value: bool) {
        self.reg.write_bit(0, value)
    }
    pub fn sd_data(&self) -> bool {
        self.reg.read_bit(1)
    }
    pub fn write_sd_data(&self, value: bool) {
        self.reg.write_bit(1, value)
    }
    pub fn si_data(&self) -> bool {
        self.reg.read_bit(2)
    }
    pub fn write_si_data(&self, value: bool) {
        self.reg.write_bit(2, value)
    }
    pub fn so_data(&self) -> bool {
        self.reg.read_bit(3)
    }
    pub fn write_so_data(&self, value: bool) {
        self.reg.write_bit(3, value)
    }

    pub fn sc_is_output(&self) -> bool {
        self.reg.read_bit(4)
    }
    pub fn set_sc_direction(&self, is_output: bool) {
        self.reg.write_bit(4, is_output)
    }
    pub fn sd_is_output(&self) -> bool {
        self.reg.read_bit(5)
    }
    pub fn set_sd_direction(&self, is_output: bool) {
        self.reg.write_bit(5, is_output)
    }
    pub fn si_is_output(&self) -> bool {
        self.reg.read_bit(6)
    }
    pub fn set_si_direction(&self, is_output: bool) {
        self.reg.write_bit(6, is_output)
    }
    pub fn so_is_output(&self) -> bool {
        self.reg.read_bit(7)
    }
    pub fn set_so_direction(&self, is_output: bool) {
        self.reg.write_bit(7, is_output)
    }

    pub fn serial_line_directions(&self) -> (bool, bool, bool, bool) {
        let value = self.reg.read();
        let masked = value & (0xF << 4);
        (
            masked & (1 << 4) != 0,
            masked & (1 << 5) != 0,
            masked & (1 << 6) != 0,
            masked & (1 << 7) != 0,
        )
    }

    pub fn write_directions(
        &self,
        sc_output: bool,
        sd_output: bool,
        si_output: bool,
        so_output: bool,
    ) {
        let old = self.reg.read();
        let masked = old & !(0xF << 4);
        let dirmask = ((sc_output as u16) << 4)
            | ((sd_output as u16) << 5)
            | ((si_output as u16) << 6)
            | ((so_output as u16) << 7);
        let new = masked | dirmask;
        self.reg.write(new);
    }
    pub fn si_interrupt_enabled(&self) -> bool {
        self.reg.read_bit(8)
    }
    pub fn enable_si_interrupt(&self, enable: bool) {
        self.reg.write_bit(8, enable)
    }

    pub fn set_mode(&self, mode: SerialMode) {
        let (fourteen, fifteen) = match mode {
            SerialMode::Joybus => (true, true),
            SerialMode::Gpio => (false, true),
            _ => (false, false),
        };
        self.reg.write_bit(14, fourteen);
        self.reg.write_bit(15, fifteen);
    }
    pub fn mode(&self) -> Option<SerialMode> {
        let final_bit = self.reg.read_bit(15);
        let second_last = self.reg.read_bit(14);
        match (second_last, final_bit) {
            (_, false) => None,
            (true, true) => Some(SerialMode::Joybus),
            (false, true) => Some(SerialMode::Gpio),
        }
    }
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

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum SerialMode {
    Normal,
    Multiplayer,
    Uart,
    Joybus,
    Gpio,
}

pub mod generalpurpose;
pub mod multiplayer;

pub struct SiocntWrapper {
    reg: RegisterWrapper,
}

method_wraps!(SiocntWrapper, reg, RegisterWrapper);

impl SiocntWrapper {
    const fn new() -> Self {
        Self {
            reg: RegisterWrapper::new(SIOCNT),
        }
    }
    pub const fn get() -> Self {
        Self::new()
    }
    pub fn mode(&self) -> SerialMode {
        let value = self.reg.read();
        if read_bit(value, 13) {
            SerialMode::Normal
        } else if read_bit(value, 12) {
            SerialMode::Uart
        } else {
            SerialMode::Multiplayer
        }
    }
    pub fn set_mode(&self, mode: SerialMode) {
        let prev = self.reg.read();
        let next = match mode {
            SerialMode::Normal => write_bit(prev, 13, false),
            SerialMode::Multiplayer => write_bit(write_bit(prev, 12, false), 13, true),
            SerialMode::Uart => write_bit(write_bit(prev, 12, true), 13, true),
            _ => prev,
        };
        self.reg.write(next);
    }
    pub fn irq_enabled(&self) -> bool {
        self.reg.read_bit(14)
    }
    pub fn enable_irq(&self, v: bool) {
        self.reg.write_bit(14, v)
    }
}

/*
===============
= NORMAL MODE =
===============
  Bit   Expl.
  0     Shift Clock (SC)        (0=External, 1=Internal)
  1     Internal Shift Clock    (0=256KHz, 1=2MHz)
  2     SI State (opponents SO) (0=Low, 1=High/None) --- (Read Only)
  3     SO during inactivity    (0=Low, 1=High) (applied ONLY when Bit7=0)
  4-6   Not used                (Read only, always 0 ???)
  7     Start Bit               (0=Inactive/Ready, 1=Start/Active)
  8-11  Not used                (R/W, should be 0)
  12    Transfer Length         (0=8bit, 1=32bit)
  13    Must be "0" for Normal Mode
  14    IRQ Enable              (0=Disable, 1=Want IRQ upon completion)
  15    Not used                (Read only, always 0)


  */
/*
=============
= UART MODE =
=============

  Bit   Expl.
  0-1   Baud Rate  (0-3: 9600,38400,57600,115200 bps)
  2     CTS Flag   (0=Send always/blindly, 1=Send only when SC=LOW)
  3     Parity Control (0=Even, 1=Odd)
  4     Send Data Flag      (0=Not Full,  1=Full)    (Read Only)
  5     Receive Data Flag   (0=Not Empty, 1=Empty)   (Read Only)
  6     Error Flag          (0=No Error,  1=Error)   (Read Only)
  7     Data Length         (0=7bits,   1=8bits)
  8     FIFO Enable Flag    (0=Disable, 1=Enable)
  9     Parity Enable Flag  (0=Disable, 1=Enable)
  10    Send Enable Flag    (0=Disable, 1=Enable)
  11    Receive Enable Flag (0=Disable, 1=Enable)
  12    Must be "1" for UART mode
  13    Must be "1" for UART mode
  14    IRQ Enable          (0=Disable, 1=IRQ when any Bit 4/5/6 become set)
  15    Not used            (Read only, always 0)

*/
