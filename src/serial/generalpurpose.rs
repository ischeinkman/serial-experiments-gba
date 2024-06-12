use core::marker::PhantomData;

use crate::utils::{read_bit_u8, write_bit_u8};

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(u8)]
pub enum GpioDirection {
    #[default]
    Input = 0,
    Output = 1,
}

impl GpioDirection {
    pub const fn from_is_output(is_output: bool) -> Self {
        if is_output {
            GpioDirection::Output
        } else {
            GpioDirection::Input
        }
    }
    pub const fn is_output(&self) -> bool {
        matches!(self, GpioDirection::Output)
    }
    pub const fn is_input(&self) -> bool {
        !self.is_output()
    }
    const fn as_u16(&self) -> u16 {
        *self as u8 as u16
    }
}

pub struct GeneralPurpose<'a> {
    _handle: PhantomData<&'a mut Serial>,
}

impl<'a> GeneralPurpose<'a> {
    pub fn new(_handle: &'a mut Serial) -> Self {
        RcntWrapper::get().set_mode(SerialMode::Gpio);
        Self {
            _handle: PhantomData,
        }
    }
    pub fn from_handle<'b: 'a>(_handle: &'a mut PhantomData<&'b mut Serial>) -> Self {
        Self {
            _handle: PhantomData,
        }
    }
    pub fn gpio_config(&self) -> GpioConfig {
        GpioConfig::from_rcnt(RcntWrapper::new().read())
    }
    pub fn set_gpio_config(&self, cfg: GpioConfig) {
        let rcnt = RcntWrapper::get();
        let old = rcnt.read();
        let masked = old & !GpioConfig::MASK;
        let new = masked | cfg.into_rcnt();
        rcnt.write(new)
    }
    pub fn interupt_enabled(&self) -> bool {
        RcntWrapper::get().si_interrupt_enabled()
    }
    pub fn set_interrupt(&self, interupt: bool) {
        RcntWrapper::get().enable_si_interrupt(interupt)
    }
    pub fn pins(&self) -> PinState {
        PinState::from_rcnt(RcntWrapper::get().read())
    }
    pub fn write_pins(&self, state: PinState) {
        let old = RcntWrapper::get().read();
        let new = (old & !PinState::MASK) | state.into_rcnt();
        RcntWrapper::get().write(new)
    }
    pub fn write_pin(&self, pin: Pin, high: bool) {
        RcntWrapper::get().write_bit(pin as u8, high)
    }
    pub fn read_pin(&self, pin: Pin) -> bool {
        RcntWrapper::get().read_bit(pin as u8)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct GpioConfig {
    pub sc: GpioDirection,
    pub sd: GpioDirection,
    pub si: GpioDirection,
    pub so: GpioDirection,
}

impl GpioConfig {
    const MASK: u16 = 0xFu16 << 4;
    const fn from_rcnt(value: u16) -> Self {
        let sc = GpioDirection::from_is_output((value & (1 << 4)) != 0);
        let sd = GpioDirection::from_is_output((value & (1 << 5)) != 0);
        let si = GpioDirection::from_is_output((value & (1 << 6)) != 0);
        let so = GpioDirection::from_is_output((value & (1 << 7)) != 0);
        Self { sc, sd, si, so }
    }
    const fn into_rcnt(self) -> u16 {
        (self.sc.as_u16() << 4)
            | (self.sd.as_u16() << 5)
            | (self.si.as_u16() << 6)
            | (self.so.as_u16() << 7)
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct PinState {
    state: u8,
}

impl PinState {
    const MASK: u16 = 0xF;
    const fn from_rcnt(rcnt: u16) -> Self {
        let masked = (rcnt & Self::MASK) as u8;
        Self { state: masked }
    }
    const fn into_rcnt(self) -> u16 {
        self.state as u16
    }
    pub const fn sc(&self) -> bool {
        read_bit_u8(self.state, 0)
    }
    pub fn set_sc(&mut self, value: bool) {
        self.state = write_bit_u8(self.state, 0, value);
    }
    pub const fn with_sc(mut self, value: bool) -> Self {
        self.state = write_bit_u8(self.state, 0, value);
        self
    }
    pub const fn sd(&self) -> bool {
        read_bit_u8(self.state, 1)
    }
    pub fn set_sd(&mut self, value: bool) {
        self.state = write_bit_u8(self.state, 1, value);
    }
    pub const fn with_sd(mut self, value: bool) -> Self {
        self.state = write_bit_u8(self.state, 1, value);
        self
    }
    pub const fn si(&self) -> bool {
        read_bit_u8(self.state, 2)
    }
    pub fn set_si(&mut self, value: bool) {
        self.state = write_bit_u8(self.state, 2, value);
    }
    pub const fn with_si(mut self, value: bool) -> Self {
        self.state = write_bit_u8(self.state, 2, value);
        self
    }
    pub const fn so(&self) -> bool {
        read_bit_u8(self.state, 3)
    }
    pub fn set_so(&mut self, value: bool) {
        self.state = write_bit_u8(self.state, 3, value);
    }
    pub const fn with_so(mut self, value: bool) -> Self {
        self.state = write_bit_u8(self.state, 3, value);
        self
    }
}

pub struct GpioState {
    pub pins: PinState,
    pub config: GpioConfig,
}

impl GpioState {
    const fn from_rcnt(rcnt: u16) -> Self {
        Self {
            pins: PinState::from_rcnt(rcnt),
            config: GpioConfig::from_rcnt(rcnt),
        }
    }
    const fn into_rcnt(&self) -> u16 {
        self.pins.into_rcnt() | self.config.into_rcnt()
    }
}
