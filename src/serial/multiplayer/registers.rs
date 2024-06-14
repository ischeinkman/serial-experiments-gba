use super::PlayerId;
use voladdress::{Safe, VolAddress};

const SIOMULTI0: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000120) };
const SIOMULTI1: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000122) };
const SIOMULTI2: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000124) };
const SIOMULTI3: VolAddress<u16, Safe, Safe> = unsafe { VolAddress::new(0x4000126) };

/// A register used for reading data sent by other GBA units in multiplayer
/// mode. 
/// 
/// Each player has a specific memory location between `0x4000120..0x4000128`
/// that the value they place in their `SIOMLT_SEND` register gets written to.
/// This location is consistent across all GBAs connected in a session; as such,
/// all GBAs will see the value that Player 3 sent in the same memory location. 
pub struct MultiplayerCommReg {
    reg: VolAddress<u16, Safe, Safe>,
}

impl MultiplayerCommReg {
    /// The register for Player 0, also known as the "parent".
    pub const P0: Self = MultiplayerCommReg::new(PlayerId::P0);
    /// The register for Player 1.
    pub const P1: Self = MultiplayerCommReg::new(PlayerId::P1);
    /// The register for Player 2.
    pub const P2: Self = MultiplayerCommReg::new(PlayerId::P2);
    /// The register for Player 3.
    pub const P3: Self = MultiplayerCommReg::new(PlayerId::P3);

    /// An array of all multiplayer communication registers for easy iteration. 
    pub const ALL: [Self; 4] = [Self::P0, Self::P1, Self::P2, Self::P3];

    /// Constructs a new `MultiplayerCommReg`. 
    /// 
    /// Note that this should never be called directly, and instead registers
    /// should be used via one of `MultiplayerCommReg::get`,
    /// `MultiplayerCommReg::ALL`, or one of the direct associated constants;
    /// all using `MultiplayerCommReg::new` will do is waste some extra memory
    /// to keep a value on the stack. 
    const fn new(player_id: PlayerId) -> Self {
        let reg = match player_id {
            PlayerId::P0 => SIOMULTI0,
            PlayerId::P1 => SIOMULTI1,
            PlayerId::P2 => SIOMULTI2,
            PlayerId::P3 => SIOMULTI3,
        };
        Self { reg }
    }
    pub const fn get(player_id: PlayerId) -> &'static Self {
        &Self::ALL[player_id as usize]
    }

    pub fn read(&self) -> Option<u16> {
        let raw = self.raw_read();
        if raw == 0xFFFF {
            None
        } else {
            Some(raw)
        }
    }
    pub fn raw_read(&self) -> u16 {
        self.reg.read()
    }
}
