use core::fmt;

use agb::external::portable_atomic::{AtomicU16, Ordering};
use agb::mgba::{self, DebugLevel, Mgba};

pub struct Logger {
    framecounter: AtomicU16,
    id: AtomicU16,
}

static LOGGER: Logger = Logger {
    framecounter: AtomicU16::new(0),
    id: AtomicU16::new(0),
};

impl Logger {
    pub fn get() -> &'static Logger {
        &LOGGER
    }
    pub fn set_level(&self, level: DebugLevel) {
        if let Some(mut mgba) = mgba::Mgba::new() {
            mgba.set_level(level);
        }
    }
    pub fn id_from_framecount(&self) -> Result<(), u16> {
        self.set_id(self.framecounter.load(Ordering::Relaxed))
    }
    pub fn set_id(&self, id: u16) -> Result<(), u16> {
        self.id
            .compare_exchange(0, id, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
    }
    pub fn tick(&self) {
        self.framecounter.fetch_add(1, Ordering::Relaxed);
    }
    pub fn log(&self, level: DebugLevel, msg: fmt::Arguments) -> Result<(), fmt::Error> {
        use DebugLevel::*;
        let Some(mut mgba) = Mgba::new() else {
            return Ok(());
        };
        let mapped_level = match level {
            Fatal => "FATAL",
            Error => "ERROR",
            Warning => "WARN ",
            Info => "INFO ",
            Debug => "DEBUG",
        };
        mgba.print(
            format_args!(
                "[{:010}] [{:03}] [{}] {}",
                self.framecounter.load(Ordering::Acquire),
                self.id.load(Ordering::Acquire),
                mapped_level,
                msg
            ),
            level,
        )
    }
}

#[macro_export]
macro_rules! debug {
    ( $( $x:expr ),*) => {{
        let _ = $crate::logs::Logger::get().log(
            agb::mgba::DebugLevel::Debug,
            format_args!($($x,)*)
        );
    }};
}

#[macro_export]
macro_rules! info {
    ( $( $x:expr ),*) => {{
        let _ = $crate::logs::Logger::get().log(
            agb::mgba::DebugLevel::Info,
            format_args!($($x,)*)
        );
    }};
}

#[macro_export]
macro_rules! warning {
    ( $( $x:expr ),*) => {{
        let _ = $crate::logs::Logger::get().log(
            agb::mgba::DebugLevel::Warning,
            format_args!($($x,)*)
        );
    }};
}

#[macro_export]
macro_rules! println {
    ( $( $x:expr ),*) => {{
        let _ = $crate::logs::Logger::get().log(
            agb::mgba::DebugLevel::Info,
            format_args!($($x,)*)
        );
    }};
}

pub use debug;
pub use info;
pub use println;
pub use warning;
