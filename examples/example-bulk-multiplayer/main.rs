//! Example to demonstrate using this crate's "bulk transfer" multiplayer
//! functionality, which is available under [crate::serial::multiplayer::bulk].
//!
//! This demonstrates basic communication between up to 4 GBAs, with input being
//! provided via button presses and output being placed on `mgba`'s logging
//! funcitonality.

#![no_std]
#![no_main]

extern crate alloc;

use agb::{
    input::{Button, ButtonController},
    interrupt::{add_interrupt_handler, Interrupt},
    mgba::DebugLevel,
    Gba,
};
use alloc::collections::VecDeque;
use alloc::format;
use core::fmt::Write;
mod logs;
use logs::Logger;
use serial_experiments_gba::serial::multiplayer::{BaudRate, MultiplayerSerial, PlayerId};
use serial_experiments_gba::serial::Serial;

#[agb::entry]
fn main(mut gba: agb::Gba) -> ! {
    multiplayer_test_main(gba)
}

fn multiplayer_test_main(mut _gba: Gba) -> ! {
    // The example emits all output to mgba's console; as such we panic in other
    // environments.
    agb::mgba::Mgba::new().expect("Should be in mgba");
    Logger::get().set_level(DebugLevel::Debug);

    let mut btns = ButtonController::new();

    // Initialize our logger with an ID to more easily see the different GBAs'
    // messages in stdout.
    println!("Now waiting for press.");
    while !btns.is_pressed(Button::A) {
        btns.update();
        Logger::get().tick();
    }
    Logger::get().id_from_framecount().unwrap();
    let _vblank_handle =
        unsafe { add_interrupt_handler(Interrupt::VBlank, |_cs| Logger::get().tick()) };

    // Enter bulk multiplayer mode.
    //
    // Copying the style from `agb` we create a top-level handle to the serial
    // port peripheral token (`Serial`) and then pass that into the lower-level
    // `MultiplayerSerial` wrapper and then finally we tell the multiplayer
    // serial wrapper to "enter bulk mode".
    let mut serial = Serial::new();
    let mut multiplayer_handle = {
        let multiplayer_handle = MultiplayerSerial::new(&mut serial, BaudRate::B9600).unwrap();

        // We initialize the bulk mode to allow for up to 128 words to be stored
        // per player, as well as being able to hold up to 128 words in the
        // message queue.
        //
        // Note that since multiplayer words are 2 bytes long and we have 5
        // buffers we're managing (1 per player + the outbox), our specifying a
        // buffer size of 128 translates to using about 1.3 KB of heap space
        // total.
        multiplayer_handle.enable_bulk_mode(128).unwrap()
    };

    // Prevent anyone in the session from initiating a transfer until we,
    // ourselves, have data in the queue to send. This is technically default
    // behaviour and therefore unecessary, but we add this line to show what's
    // possible.
    multiplayer_handle.block_transfers_until_have_data(true);

    println!("Entered multiplayer mode");
    println!("We are {:?}", multiplayer_handle.id());
    let mut loopcnt = 0;
    let mut queues = [const { VecDeque::new() }; 4];
    loop {
        // You need to call `BulkMultiplayer::tick` at least once per frame for
        // general maintenance tasks. This will likely be done at a similar time
        // to your other maintenance items, such as `mixer.tick()`,
        // `button_controller.update()`, etc.
        //
        // Note that this function will not take up much CPU time, and can
        // therefore be called whenever in the update loop.
        multiplayer_handle.tick().unwrap();

        // Queue out our next message to send to the rest of the session.
        btns.update();
        multiplayer_handle
            .queue_send(&write_buttons(&btns))
            .unwrap();
        println!("Queued send buffer.");

        let mut msg = format!("Current loop: {:03} \n", loopcnt,);

        // Skip any transfers where we didn't see any data from anyone but
        // ourselves; this can happen if, for example, we're the parent unit and
        // the children aren't blocking us or if the children are sending NULL
        // bytes (for example, to initialize their IDs).
        let skipped = multiplayer_handle.skip_empty_transfers();
        println!("Skipped {} empty transfers.", skipped);

        // Pull in some data from the queue.
        //
        // Generally one would assume we'd see `WORDS_PER_BLOCK` words at a
        // time; however, if we're missing a sibling or out of sync with the
        // parent we could end up with either NULL bytes or partial messages
        // being in the queue when we're reading it. To get around this we pull
        // in data into our own separate VecDeque instances that we can parse at
        // our leisure.
        //
        // This is generally a good practice since it allows an easier time
        // pulling and parsing larger messages.
        let mut p0_buff = [0; 10];
        let mut p1_buff = [0; 10];
        let mut p2_buff = [0; 10];
        let mut p3_buff = [0; 10];
        let mut buffers = [
            p0_buff.as_mut_slice(),
            p1_buff.as_mut_slice(),
            p2_buff.as_mut_slice(),
            p3_buff.as_mut_slice(),
        ];
        multiplayer_handle.read_bulk(&mut buffers).unwrap();
        println!("RAW BUFFERS:");
        for (idx, b) in buffers.iter().enumerate() {
            if idx == multiplayer_handle.id() as usize {
                println!("    {:0X?} | SELF", b);
            } else {
                println!("    {:0X?}", b);
            }
        }
        for pid in PlayerId::ALL {
            let buf = &buffers[pid as usize];
            let que = &mut queues[pid as usize];

            // Add the new data into the parsing queue
            que.extend(buf.iter().copied());

            // Skip any NULL bytes, meaning the unit didn't send any data.
            while que.front().copied() == Some(0xFFFF) {
                que.pop_front();
            }

            if pid == multiplayer_handle.id() {
                write!(&mut msg, " - Player {} (Self): ", pid as u8).ok();
            } else {
                write!(&mut msg, " - Player {}       : ", pid as u8).ok();
            }

            if que.len() >= WORDS_PER_BLOCK {
                // We have a full message; parse it.
                let mut buf = [0x0000; WORDS_PER_BLOCK];
                for slot in buf.iter_mut() {
                    *slot = que.pop_front().unwrap();
                }
                write!(&mut msg, "{:?}", &buf).ok();
                writeln!(&mut msg, " => {:?}", parse_buttons(&buf)).ok();
            } else {
                writeln!(&mut msg, "Queue size is only {}", que.len()).ok();
            }
        }
        println!("{}", msg);
        loopcnt += 1;
    }
    drop(_vblank_handle);
}

/// Basic communication protocol.
///
/// Summary:
/// * Each message consists of 9 words -- 1 word for each button and 1 sentinel.
/// * If a button is pressed, its word is set to a value of 0x764e; otherwise,
///   it is set to 0xfa15.
///     * These values were chosen since they look like "true" and "false",
///       respectively.
mod protocol {
    extern crate alloc;

    use agb::input::{Button, ButtonController};

    use alloc::vec::Vec;

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
    pub const WORDS_PER_BLOCK: usize = 1 + TO_CHECK.len();
    pub const END_BLOCK_SENTINEL: u16 = 0xE4D;
    pub const TRUE_WORD: u16 = 0x764e;
    pub const FALSE_WORD: u16 = 0xFA15;

    /// Parses a message into the list of buttons currently pressed.
    ///
    /// Panics on invalid message. This includes:
    /// * The message not ending with [END_BLOCK_SENTINEL]
    /// * The message otherwise containing a value other than [TRUE_WORD] or
    ///   [FALSE_WORD]
    pub fn parse_buttons(n: &[u16; WORDS_PER_BLOCK]) -> Vec<Button> {
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

    pub fn write_buttons(btns: &ButtonController) -> [u16; WORDS_PER_BLOCK] {
        let mut n = [END_BLOCK_SENTINEL; WORDS_PER_BLOCK];
        for (idx, btn) in TO_CHECK.iter().enumerate() {
            let state = btns.is_pressed(*btn);
            let _edge = btns.is_just_pressed(*btn);
            n[idx] = if state { TRUE_WORD } else { FALSE_WORD };
        }
        n
    }
}
use protocol::*;
