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
    display::{
        object::OamManaged,
        tiled::{MapLoan, RegularBackgroundSize, RegularMap, TiledMap, VRamManager},
        Priority,
    },
    external::portable_atomic::Ordering,
    input::{Button, ButtonController},
    interrupt::{add_interrupt_handler, Interrupt},
    mgba::DebugLevel,
    Gba,
};

use alloc::{format, vec::Vec};
use core::fmt::Write;
use logs::{println, warning, Logger};
use serial_experiments_gba::*;
pub use utils::*;

// The main function must take 1 arguments and never return. The agb::entry decorator
// ensures that everything is in order. `agb` will call this after setting up the stack
// and interrupt handlers correctly. It will also handle creating the `Gba` struct for you.
#[agb::entry]
fn main(mut gba: agb::Gba) -> ! {
    multiplayer_test_main(gba)
}

use serial::{
    multiplayer::{MultiplayerSerial, PlayerId, TransferError},
    BaudRate, Serial,
};

#[allow(dead_code)]
fn multiplayer_test_main(mut _gba: Gba) -> ! {
    agb::mgba::Mgba::new().expect("Should be in mgba");
    Logger::get().set_level(DebugLevel::Debug);
    let mut btns = ButtonController::new();
    let to_check = [
        Button::UP,
        Button::DOWN,
        Button::LEFT,
        Button::RIGHT,
        Button::A,
        Button::B,
        Button::L,
        Button::R,
    ];

    println!("Now waiting for press.");
    while !btns.is_pressed(Button::A) {
        btns.update();
        Logger::get().tick();
    }
    Logger::get().id_from_framecount().unwrap();
    let mut serial = Serial::new();
    let mut multiplayer_handle = MultiplayerSerial::new(&mut serial, BaudRate::B9600).unwrap();
    multiplayer_handle.enable_buffer_interrupt();
    println!("Entered multiplayer mode");
    multiplayer_handle.initialize_id().unwrap();
    println!("We are {:?}", multiplayer_handle.id().unwrap());

    let _vblank_handle =
        unsafe { add_interrupt_handler(Interrupt::VBlank, |_cs| Logger::get().tick()) };
    let mut loopcnt = 0;
    loop {
        multiplayer_handle.mark_unready();
        btns.update();
        let mut n = 0u16;
        for (idx, btn) in to_check.into_iter().enumerate() {
            let state = btns.is_pressed(btn);
            let edge = btns.is_just_pressed(btn);
            n |= ((state as u16) << idx) | ((edge as u16) << (idx + 8));
        }
        multiplayer_handle.write_send_reg(n);
        multiplayer_handle.mark_ready();

        while !multiplayer_handle.all_ready() {}
        match multiplayer_handle.start_transfer() {
            Ok(()) => {}
            Err(TransferError::AlreadyInProgress) => {
                warning!("Already in progress");
            }
            Err(e) => {
                panic!("{:?}", e);
            }
        }
        let mut msg = format!("Current loop: {:03} \n", loopcnt,);
        for pid in PlayerId::ALL {
            write!(&mut msg, "  -  Player {}", pid as u8).ok();
            if Some(pid) == multiplayer_handle.id() {
                write!(&mut msg, "(Self)").ok();
            } else {
                write!(&mut msg, "      ").ok();
            }
            writeln!(
                &mut msg,
                ": {:0x}",
                multiplayer_handle
                    .read_player_reg_raw(pid)
                    .unwrap_or(0xFFFF)
            )
            .ok();
        }
        println!("{}", msg);
        loopcnt += 1;
    }
    drop(_vblank_handle);
}
