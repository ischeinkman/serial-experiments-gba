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
const WORDS_PER_BLOCK: usize = 1 + TO_CHECK.len();
const END_BLOCK_SENTINEL: u16 = 0xE4D;
const TRUE_WORD: u16 = 0x764e;
const FALSE_WORD: u16 = 0xFA15;

fn parse_buttons(n: &[u16; WORDS_PER_BLOCK]) -> Vec<Button> {
    assert_eq!(
        n[WORDS_PER_BLOCK - 1],
        END_BLOCK_SENTINEL,
        "Expected: {:X}, actual: {:X} (buff: {:X?})",
        END_BLOCK_SENTINEL,
        n[WORDS_PER_BLOCK - 1],
        n
    );
    let mut retvl = Vec::new();
    for (idx, btn) in TO_CHECK.iter().enumerate() {
        match n[idx] {
            a if a == TRUE_WORD => retvl.push(*btn),
            a if a == FALSE_WORD => {}
            other => {
                panic!("Found unexpected word: {:x}", other);
            }
        }
    }
    retvl
}

fn write_buttons(btns: &ButtonController) -> [u16; WORDS_PER_BLOCK] {
    let mut n = [END_BLOCK_SENTINEL; WORDS_PER_BLOCK];
    for (idx, btn) in TO_CHECK.iter().enumerate() {
        let state = btns.is_pressed(*btn);
        let _edge = btns.is_just_pressed(*btn);
        n[idx] = if state { TRUE_WORD } else { FALSE_WORD };
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
            .queue_send(&write_buttons(&btns))
            .unwrap();
        println!("Queued send buffer.");

        let mut msg = format!("Current loop: {:03} \n", loopcnt,);
        let mut p0_buff = [0xFFFFu16; WORDS_PER_BLOCK];
        let mut p1_buff = [0xFFFFu16; WORDS_PER_BLOCK];
        let mut p2_buff = [0xFFFFu16; WORDS_PER_BLOCK];
        let mut p3_buff = [0xFFFFu16; WORDS_PER_BLOCK];
        let mut buffers = [
            p0_buff.as_mut_slice(),
            p1_buff.as_mut_slice(),
            p2_buff.as_mut_slice(),
            p3_buff.as_mut_slice(),
        ];
        multiplayer_handle.read_all(&mut buffers).unwrap();

        for pid in PlayerId::ALL {
            write!(&mut msg, "  -  Player {}", pid as u8).ok();
            if pid == multiplayer_handle.id() {
                write!(&mut msg, "(Self)").ok();
            } else {
                write!(&mut msg, "      ").ok();
            }
            write!(&mut msg, ": ?? // ").ok();
            let buf = &[p0_buff, p1_buff, p2_buff, p3_buff][pid as usize];
            write!(&mut msg, "{:?}", &buf).ok();
            writeln!(&mut msg, " => {:?}", parse_buttons(buf)).ok();
        }
        loopcnt += 1;
    }
    drop(_vblank_handle);
}
