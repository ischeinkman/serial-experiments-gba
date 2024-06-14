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

static OUTBUFFER: GbaCell<Ringbuffer> = GbaCell::new(Ringbuffer::empty());

/// If true, all data transfers for all other GBAs in the session will be blocked until we ourselves also write data to be sent out.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkTickError {
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
        inner.initialize_id()?;
        let nbuff = TransferBuffer::new(cap);
        if BUFFER_SLOT
            .swap_if(nbuff, |old| old.is_placeholder())
            .is_err()
        {
            return Err(BulkInitError::AlreadyInitialized);
        }
        let nout = Ringbuffer::new(cap);
        OUTBUFFER
            .swap_if(nout, |old| old.is_placeholder())
            // Shouldn't be possible if the previous check passed, but still
            .map_err(|_| BulkInitError::AlreadyInitialized)?;
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

    pub fn read_bulk(
        &mut self,
        buffers: &mut [&mut [u16]; 4],
    ) -> Result<[usize; 4], MultiplayerError> {
        critical_section::with(|cs| {
            let tbuf = BUFFER_SLOT.swap(TransferBuffer::empty());
            Ok(tbuf.read_bulk(buffers, cs))
        })
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

    pub fn tick(&mut self) -> Result<(), BulkTickError> {
        match self.inner.start_transfer() {
            Err(TransferError::FailedOkayCheck) => Err(BulkTickError::FailedOkayCheck),
            Ok(())
            | Err(TransferError::AlreadyInProgress)
            | Err(TransferError::FailedReadyCheck) => Ok(()),
        }
    }
    pub fn read_player_reg_raw(&self, pid: PlayerId) -> u16 {
        MultiplayerCommReg::get(pid).raw_read()
    }
}

fn bulk_mode_interrupt_callback(cs: CriticalSection<'_>) {
    let siocnt = MultiplayerSiocnt::get();
    let flags = (siocnt.read() & 0xFF) as u8;
    let p0 = MultiplayerCommReg::get(PlayerId::P0).raw_read();
    let p1 = MultiplayerCommReg::get(PlayerId::P1).raw_read();
    let p2 = MultiplayerCommReg::get(PlayerId::P2).raw_read();
    let p3 = MultiplayerCommReg::get(PlayerId::P3).raw_read();

    if !(p0 == SENTINEL && p1 == SENTINEL && p2 == SENTINEL && p3 == SENTINEL) {
        // We're already in a critical section, so this won't break anything.
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
            if BLOCK_TRANSFER_UNTIL_SEND.get_copy() {
                mark_unready()
            }
        }
    });
}
