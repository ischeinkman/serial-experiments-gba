// Games made using `agb` are no_std which means you don't have access to the standard
// rust library. This is because the game boy advance doesn't really have an operating
// system, so most of the content of the standard library doesn't apply.
//
// Provided you haven't disabled it, agb does provide an allocator, so it is possible
// to use both the `core` and the `alloc` built in crates.
#![no_std]
// `agb` defines its own `main` function, so you must declare your game's main function
// using the #[agb::entry] proc macro. Failing to do so will cause failure in linking
// which won't be a particularly clear error message.
#![no_main]
// This is required to allow writing tests
#![cfg_attr(test, feature(custom_test_frameworks))]
#![cfg_attr(test, reexport_test_harness_main = "test_main")]
#![cfg_attr(test, test_runner(agb::test_runner::test_runner))]
#![allow(clippy::assertions_on_constants)]

extern crate alloc;

use agb::{
    input::{Button, ButtonController},
    interrupt::{add_interrupt_handler, Interrupt},
    mgba::DebugLevel,
    Gba,
};

use alloc::{format, vec::Vec};
use core::fmt::Write;
use logs::Logger;
use serial_experiments_gba::*;
pub use utils::*;
mod logs;

// The main function must take 1 arguments and never return. The agb::entry decorator
// ensures that everything is in order. `agb` will call this after setting up the stack
// and interrupt handlers correctly. It will also handle creating the `Gba` struct for you.
#[agb::entry]
fn main(mut gba: agb::Gba) -> ! {
    multiplayer_test_main(gba)
}

use serial::{
    multiplayer::{BaudRate, MultiplayerSerial, PlayerId},
    Serial,
};
const TO_CHECK: &[Button] = &[
    Button::UP,
    Button::DOWN,
    Button::LEFT,
    Button::RIGHT,
    Button::A,
    Button::B,
    Button::L,
    Button::R,
];

fn parse_buttons(n: u16) -> Vec<Button> {
    let mut retvl = Vec::new();
    for (idx, btn) in TO_CHECK.iter().enumerate() {
        if read_bit(n, idx as u8) {
            retvl.push(*btn);
        }
    }
    retvl
}

fn write_buttons(btns: &ButtonController) -> u16 {
    let mut n = 0u16;
    for (idx, btn) in TO_CHECK.iter().enumerate() {
        let idx = idx as u8;
        let state = btns.is_pressed(*btn);
        let edge = btns.is_just_pressed(*btn);
        n = write_bit(n, idx, state);
        n = write_bit(n, idx + 8, edge);
    }
    n
}

fn multiplayer_test_main(mut _gba: Gba) -> ! {
    agb::mgba::Mgba::new().expect("Should be in mgba");
    Logger::get().set_level(DebugLevel::Debug);
    let mut btns = ButtonController::new();

    println!("Now waiting for press.");
    while !btns.is_pressed(Button::A) {
        btns.update();
        Logger::get().tick();
    }
    Logger::get().id_from_framecount().unwrap();
    let mut serial = Serial::new();

    let mut multiplayer_handle = {
        let multiplayer_handle = MultiplayerSerial::new(&mut serial, BaudRate::B9600).unwrap();
        multiplayer_handle.enable_bulk_mode(128).unwrap()
    };
    multiplayer_handle.block_transfers_until_have_data(true);
    println!("Entered multiplayer mode");
    println!("We are {:?}", multiplayer_handle.id());

    let _vblank_handle =
        unsafe { add_interrupt_handler(Interrupt::VBlank, |_cs| Logger::get().tick()) };
    let mut loopcnt = 0;
    loop {
        btns.update();
        multiplayer_handle.tick().unwrap();
        multiplayer_handle
            .queue_send(&[write_buttons(&btns)])
            .unwrap();

        let mut msg = format!("Current loop: {:03} \n", loopcnt,);
        let mut buffers = [
            &mut [0xFFFFu16; 128][..],
            &mut [0xFFFFu16; 128][..],
            &mut [0xFFFFu16; 128][..],
            &mut [0xFFFFu16; 128][..],
        ];
        let readcounts = multiplayer_handle.read_bulk(&mut buffers).unwrap();
        for pid in PlayerId::ALL {
            write!(&mut msg, "  -  Player {}", pid as u8).ok();
            if pid == multiplayer_handle.id() {
                write!(&mut msg, "(Self)").ok();
            } else {
                write!(&mut msg, "      ").ok();
            }
            write!(&mut msg, ": ?? // ").ok();
            let read = readcounts[pid as usize];
            let buf = &buffers[pid as usize][..read];
            writeln!(&mut msg, "{} - {:?}", read, &buf).ok();
            for (idx, n) in buf.iter().enumerate() {
                writeln!(&mut msg, "        {:02}: {:?}", idx, parse_buttons(*n)).ok();
            }
        }
        if readcounts.iter().any(|n| *n > 0) {
            println!("{}", msg);
        }
        loopcnt += 1;
    }
    drop(_vblank_handle);
}