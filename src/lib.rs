#![no_std]
#![no_main]
// This is required to allow writing tests
#![cfg_attr(test, feature(custom_test_frameworks))]
#![cfg_attr(test, reexport_test_harness_main = "test_main")]
#![cfg_attr(test, test_runner(agb::test_runner::test_runner))]
#![allow(clippy::assertions_on_constants)]

mod serial;
pub use serial::*;
pub mod utils;

extern crate alloc;

/// Needed to get `agb`'s test harness to work.
#[cfg(test)]
#[agb::entry]
fn main(mut gba: agb::Gba) -> ! {
    loop {}
}
