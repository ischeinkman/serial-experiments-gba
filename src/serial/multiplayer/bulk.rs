//! Provides a higher-level easy-to-use interface for dealing with multiplayer
//! communication on the GBA.
//!
//! # Basic Usage
//!
//! 1. Start "bulk multiplayer" mode by calling [BulkMultiplayer::new]. This
//!    will initialize the player IDs, allocated the read & write buffers, and
//!    configure the necessary interrupts.
//! 2. Set the value of [BulkMultiplayer::block_transfers_until_have_data]
//!    depending on your usecase. If `true` (default) then transfers will only
//!    happen when EVERY SINGLE unit in the session has data available to send;
//!    otherwise transfers will happen when ANY unit is sending data. Note that
//!    any individual unit setting this to `true` will block the transfer for
//!    all other units as well!
//! 3. Load data to be sent to other units with [BulkMultiplayer::queue_send]
//!    and/or read data other players have sent with
//!    [BulkMultiplayer::read_bulk].
//! 4. Make sure [BulkMultiplayer::tick] is called during your main game loop.
//!
//! # Notes
//! * Due to GBA hardware quirks it is impossible to distinguish between a unit
//!   not being connected and a unit sending a `0xFFFF`. As such be sure to not
//!   send that value as part of your transfer if you don't want to lose
//!   information.
//! * This mode currently assumes that all units will attempt to call
//!   [BulkMultiplayer::new] at around the same time due to some initialization
//!   quirks. While we don't expect things to break if this is not true, we
//!   cannot guarantee no data will be lost.

use agb::external::critical_section::{self, CriticalSection};
use agb::interrupt::{add_interrupt_handler, Interrupt};

use crate::utils::GbaCell;

use super::ringbuf::Ringbuffer;
use super::{
    buffer::TransferBuffer, mark_unready, MultiplayerCommReg, MultiplayerError, MultiplayerSerial,
    MultiplayerSiocnt, PlayerId, SENTINEL, SIOMLT_SEND,
};
use super::{enter_multiplayer, TransferError};

/// The data buffer to store communicated words in.
static BUFFER_SLOT: GbaCell<TransferBuffer> = GbaCell::new(TransferBuffer::empty());

/// The data buffer for words we will communicate out to the other units in the
/// session.
static OUTBUFFER: GbaCell<Ringbuffer> = GbaCell::new(Ringbuffer::empty());

/// If true, all data transfers for all other GBAs in the session will be
/// blocked until we ourselves also write data to be sent out.
static BLOCK_TRANSFER_UNTIL_SEND: GbaCell<bool> = GbaCell::new(true);

pub struct BulkMultiplayer<'a> {
    inner: MultiplayerSerial<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkInitError {
    AlreadyInitialized,
    TransferError(TransferError),
}
impl From<TransferError> for BulkInitError {
    fn from(value: TransferError) -> Self {
        BulkInitError::TransferError(value)
    }
}

/// An error that can happen during per-frame processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkTickError {
    /// The serial I/O error bit was flagged during per-frame processing.
    FailedOkayCheck,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    QueueNotEmpty,
    MultiplayerError(MultiplayerError),
}

impl From<MultiplayerError> for QueueError {
    fn from(value: MultiplayerError) -> Self {
        QueueError::MultiplayerError(value)
    }
}

impl<'a> BulkMultiplayer<'a> {
    pub fn new(mut inner: MultiplayerSerial<'a>, cap: usize) -> Result<Self, BulkInitError> {
        // Step 1 is make sure we know what player we are.
        //
        // Technically not necessary but it makes things usage easier since
        // there's no worries about whether or not we know who we are.
        initialize_id(&mut inner)?;

        // Step 2 is to initialize the static buffers.
        //
        // The total heap usage is 5 * cap; 1 inbox for each player + the outbox.
        let nbuff = TransferBuffer::new(cap);
        let nout = Ringbuffer::new(cap);
        BUFFER_SLOT
            .swap_if(nbuff, |old| old.is_placeholder())
            .map_err(|_| BulkInitError::AlreadyInitialized)?;
        OUTBUFFER
            .swap_if(nout, |old| old.is_placeholder())
            // Shouldn't be possible if the previous check passed, but still
            .map_err(|_| BulkInitError::AlreadyInitialized)?;

        // Step 3 is to set up the interrupts for reading & writing our data.
        inner.buffer_interrupt = unsafe {
            Some(add_interrupt_handler(
                Interrupt::Serial,
                bulk_mode_interrupt_callback,
            ))
        };
        inner.enable_interrupt(true);

        Ok(Self { inner })
    }

    pub fn id(&self) -> PlayerId {
        // #SAFETY
        //
        // The only way we can create a [BulkMultiplayer] instance is via
        // [BulkMultiplayer::new], which only succeeds if
        // [MultiplayerSerial::initialize_id] succeeds (which would mean
        // `self.inner.player_id` is populated).
        unsafe { self.inner.playerid.unwrap_unchecked() }
    }

    /// Pulls data from the multiplayer buffer into the provided data buffers. Returns the number of words read, per player.
    pub fn read_bulk(
        &mut self,
        buffers: &mut [&mut [u16]; 4],
    ) -> Result<[usize; 4], MultiplayerError> {
        critical_section::with(|cs| BUFFER_SLOT.lock_in(cs, |tbuf| Ok(tbuf.read_bulk(buffers, cs))))
    }
    pub fn leave(mut self) -> MultiplayerSerial<'a> {
        self.inner.enable_interrupt(false);
        self.inner.buffer_interrupt = None;
        BUFFER_SLOT.swap(TransferBuffer::empty());
        self.inner
    }

    pub fn will_block_transfers(&self) -> bool {
        BLOCK_TRANSFER_UNTIL_SEND.get_copy()
    }
    pub fn block_transfers_until_have_data(&mut self, value: bool) {
        BLOCK_TRANSFER_UNTIL_SEND.swap(value);
    }
    pub fn queue_send(&mut self, buffer: &[u16]) -> Result<usize, QueueError> {
        let res = critical_section::with(|cs| {
            OUTBUFFER.lock_in(cs, |outbuff| outbuff.write_bulk(buffer, cs))
        });
        enter_multiplayer(self.inner.rate)?;
        Ok(res)
    }

    /// Perform any per-frame maintenance required for bulk multiplayer mode.
    pub fn tick(&mut self) -> Result<(), BulkTickError> {
        match self.inner.start_transfer() {
            Err(TransferError::FailedOkayCheck) => Err(BulkTickError::FailedOkayCheck),
            Ok(())
            | Err(TransferError::AlreadyInProgress)
            | Err(TransferError::FailedReadyCheck) => Ok(()),
        }
    }
}

/// Subroutine to make sure the [PlayerId] bits are valid & set on the provided
/// [MultiplayerSerial] instance by forcing a single data transfer with a
/// sentinel value.
fn initialize_id(inner: &mut MultiplayerSerial) -> Result<(), TransferError> {
    const SENTINEL: u16 = 0xFEAD;
    inner.mark_unready();
    inner.write_send_reg(SENTINEL);
    inner.mark_ready();
    loop {
        {
            match inner.start_transfer() {
                Ok(()) => {}
                Err(TransferError::AlreadyInProgress) => {
                    // Parent beat us to it; let it keep going
                }
                Err(TransferError::FailedReadyCheck) => {
                    // Others are lagging; wait for them
                }
                Err(other) => {
                    return Err(other);
                }
            };
        }

        let reg_statuses = MultiplayerCommReg::ALL.map(|reg| reg.read());
        let my_id = MultiplayerSiocnt::get().id();
        if reg_statuses[my_id as usize] == Some(SENTINEL) {
            inner.playerid = Some(my_id);
            break;
        }
    }
    Ok(())
}

/// The interrupt callback called every time the parent unit (with
/// [PlayerId::P0]) sends data with [MultiplayerSerial::start_transfer].
fn bulk_mode_interrupt_callback(cs: CriticalSection<'_>) {
    let siocnt = MultiplayerSiocnt::get();
    let flags = (siocnt.read() & 0xFF) as u8;
    let p0 = MultiplayerCommReg::get(PlayerId::P0).raw_read();
    let p1 = MultiplayerCommReg::get(PlayerId::P1).raw_read();
    let p2 = MultiplayerCommReg::get(PlayerId::P2).raw_read();
    let p3 = MultiplayerCommReg::get(PlayerId::P3).raw_read();

    if !(p0 == SENTINEL && p1 == SENTINEL && p2 == SENTINEL && p3 == SENTINEL) {
        // This will only happen if NONE of the units had data to send,
        // INCLUDING US, and ALL of them set `block_transfers_until_have_data`
        // to `false`. In that case we'd hit this case every time the parent
        // unit hit `BulkMultiplayer::tick`, so to not waste cycles and memory
        // we don't write the all-sentinel case down.
        BUFFER_SLOT.lock_in(cs, |tbuff| {
            debug_assert!(!tbuff.is_placeholder());
            //TODO: handle error
            let _res = tbuff.push(p0, p1, p2, p3, flags, cs);
        });
    }

    OUTBUFFER.lock_in(cs, |outbuff| {
        let next = outbuff.pop(cs);
        if let Some(nxt) = next {
            SIOMLT_SEND.write(nxt);
        } else {
            SIOMLT_SEND.write(SENTINEL);
            if BLOCK_TRANSFER_UNTIL_SEND.get_copy_in(cs) {
                mark_unready()
            }
        }
    });
}
