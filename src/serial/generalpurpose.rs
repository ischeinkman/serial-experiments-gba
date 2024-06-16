//! The GBA allows the serial port to be used as a 4-pin GPIO parallel port,
//! which each pin being able to be used as either an input or an output.

use core::marker::PhantomData;

use agb::{
    external::critical_section::CriticalSection,
    interrupt::{add_interrupt_handler, Interrupt, InterruptHandler},
};

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
}

pub struct GeneralPurpose<'a> {
    _handle: PhantomData<&'a mut Serial>,
    interrupt_handle: Option<InterruptHandler>,
}

impl<'a> GeneralPurpose<'a> {
    pub fn new(_handle: &'a mut Serial) -> Self {
        RcntWrapper::get().set_mode(SerialMode::Gpio);
        Self {
            _handle: PhantomData,
            interrupt_handle: None,
        }
    }
    pub fn gpio_config(&self) -> GpioConfig {
        GpioConfig::from_rcnt(RcntWrapper::new().read())
    }
    pub fn set_gpio_config(&mut self, cfg: GpioConfig) {
        let rcnt = RcntWrapper::get();
        let old = rcnt.read();
        let masked = old & !GpioConfig::MASK;
        let new = masked | cfg.into_rcnt();
        rcnt.write(new)
    }
    pub fn interupt_enabled(&self) -> bool {
        RcntWrapper::get().si_interrupt_enabled()
    }
    pub fn enable_interrupt(&mut self, interupt: bool) {
        RcntWrapper::get().enable_si_interrupt(interupt)
    }
    /// Adds a callback that will be called whenever the SI line is set to HIGH.
    ///
    /// # Safety
    /// The callback `cb` **must not** allocate.
    pub unsafe fn set_interrupt(&mut self, cb: impl Fn(CriticalSection) + Send + Sync + 'static) {
        self.interrupt_handle = Some(add_interrupt_handler(Interrupt::Serial, cb));
    }
    /// Gets the current state of the GPIO pins.
    pub fn pins(&self) -> PinState {
        PinState::from_rcnt(RcntWrapper::get().read())
    }
    /// Writes the state of all 4 GPIO pins at once.
    pub fn write_pins(&mut self, state: PinState) {
        let old = RcntWrapper::get().read();
        let new = (old & !PinState::MASK) | state.into_rcnt();
        RcntWrapper::get().write(new)
    }
    /// Sets a pin to either HIGH or LOW.
    pub fn write_pin(&mut self, pin: Pin, high: bool) {
        RcntWrapper::get().write_bit(pin as u8, high)
    }

    /// Gets the state of the pin.
    pub fn read_pin(&self, pin: Pin) -> bool {
        RcntWrapper::get().read_bit(pin as u8)
    }

    pub fn state(&self) -> GpioState {
        GpioState::from_rcnt(RcntWrapper::get().read())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct GpioConfig {
    value: u8,
}

impl GpioConfig {
    const MASK: u16 = 0xFu16 << 4;
    pub const fn sc(&self) -> GpioDirection {
        GpioDirection::from_is_output(read_bit_u8(self.value, 4))
    }
    pub const fn with_sc(self, dir: GpioDirection) -> Self {
        let value = write_bit_u8(self.value, 4, dir.is_output());
        Self { value }
    }
    pub const fn sd(&self) -> GpioDirection {
        GpioDirection::from_is_output(read_bit_u8(self.value, 5))
    }
    pub const fn with_sd(self, dir: GpioDirection) -> Self {
        let value = write_bit_u8(self.value, 5, dir.is_output());
        Self { value }
    }
    pub const fn si(&self) -> GpioDirection {
        GpioDirection::from_is_output(read_bit_u8(self.value, 6))
    }
    pub const fn with_si(self, dir: GpioDirection) -> Self {
        let value = write_bit_u8(self.value, 6, dir.is_output());
        Self { value }
    }
    pub const fn so(&self) -> GpioDirection {
        GpioDirection::from_is_output(read_bit_u8(self.value, 7))
    }
    pub const fn with_so(self, dir: GpioDirection) -> Self {
        let value = write_bit_u8(self.value, 7, dir.is_output());
        Self { value }
    }
    const fn from_rcnt(value: u16) -> Self {
        Self {
            value: (value & Self::MASK) as u8,
        }
    }
    const fn into_rcnt(self) -> u16 {
        self.value as u16
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
    value: u8,
}

impl GpioState {
    pub const fn pins(&self) -> PinState {
        PinState::from_rcnt(self.value as u16)
    }
    pub const fn config(&self) -> GpioConfig {
        GpioConfig::from_rcnt(self.value as u16)
    }
    const fn from_rcnt(rcnt: u16) -> Self {
        Self {
            value: (rcnt & 0xFF) as u8,
        }
    }
    #[allow(unused)]
    const fn into_rcnt(self) -> u16 {
        self.value as u16
    }
}
