//! Part-record mode words: mode flags, runtime scroll flags and autoscroll.

/// Part-record word 7, copied to `$FF37A4` by `$11754`. Besides the bits
/// below, bit 4 (runtime flags bit 5) makes `$2DF4` replace the ordinary
/// plane A predraw (horse/surf overlays or the `$4AED2` boss layout) and bit 6
/// (`$FF37A5` bit 6) selects the 48x48 exit box of `$ABDE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeFlags(pub u16);
impl ModeFlags {
    /// Bits 12..15 -> `$FF39F6`; bit 0 of `$FF39F7` selects boss arenas.
    pub fn arena(self) -> u8 {
        (self.0 >> 12) as u8
    }
    /// `$8BC` boss palettes, `$2A41E` boss art, `$1300` skipped.
    pub fn boss_arena(self) -> bool {
        self.arena() & 1 != 0
    }
    /// `$A30E`: the resident puff block (`$8001C` entry 7) loads unless
    /// `$FF39F7` bit 0 (boss arena) or bit 1 (the 6-1 Karura area) is set.
    pub fn resident_puffs(self) -> bool {
        self.arena() & 3 == 0
    }
    /// Bit 7 (`$FF37A5` negative): mounted player (horse or surfboard). `$A334`
    /// loads `$3C646` instead of `$3C570`; `$377D2` installs mounted tables.
    pub fn mounted(self) -> bool {
        self.0 & 0x80 != 0
    }
    /// Bit 8 -> runtime flags bit 4, tested by `$22C4C` to raise the player
    /// sprite's priority bit.
    pub fn player_high_priority(self) -> bool {
        self.0 & 0x100 != 0
    }
    /// Bit 5: the exit loads the next part instead of streaming into it.
    /// `$11754` keeps the bit in `$FF01B0`; `Func_003400` (the end of each
    /// stage and part load) moves it to `Game_Flags` bit 11 (`$FF0028`
    /// bit 3). With it set, `$AC2E` skips the `$AC38` stream request and
    /// raises `$FF0029` bit 1, and `Mode_Game` `$B08A` then increments
    /// `Part_Index` and reloads (`$B0AA` -> `$B136`), or past the last part
    /// enters the next substage (`$B0C6`).
    pub fn reload_exit(self) -> bool {
        self.0 & 0x20 != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialMode {
    Normal,
    /// Mounted, ordinary priority: round 2-1.
    Horse,
    /// Mounted with raised player priority: round 4-1.
    Surf,
    BossArena,
}

/// `$FF0146` after `$11754` merges mode bits into record word 4. Bits 2..3
/// are the camera limit mode dispatched by `$1C66`; bit 6 makes the final
/// exit start a camera lock (`$AC22`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeFlags(pub u16);
impl RuntimeFlags {
    pub fn from_record(flags: u16, mode: u16) -> Self {
        let mut d2 = flags;
        d2 = (d2 & !0x10) | ((mode & 0x100) >> 4);
        d2 = (d2 & !0x20) | ((mode & 0x10) << 1);
        d2 = (d2 & 0x8fff) | (mode & 7).rotate_right(4);
        Self(d2)
    }
    /// Bit 0: row-major layout and vertical streaming (`$38A0`, `$2A54`).
    pub fn vertical(self) -> bool {
        self.0 & 1 != 0
    }
    /// Bit 6 (`$FF0147` bit 6): besides making the exit a camera lock
    /// (`$AC22`), `$1D4C` runs `$342E` after the scroll routine every frame,
    /// holding the player between camera X + 32 and + 288 and no higher
    /// than camera Y + 16.
    pub fn screen_clamp(self) -> bool {
        self.0 & 0x40 != 0
    }
    /// High byte & 7 is written to VDP register $0B by `$1B0E`: bits 0..1
    /// select full-screen (0), eight-line strip (2) or per-line (3) hscroll,
    /// bit 2 two-cell column vertical scroll.
    pub fn vdp_mode_3(self) -> u8 {
        (self.0 >> 8) as u8 & 7
    }
    /// `Func_0225A2`'s grounded vertical target. A byte read from `$FF0146`
    /// selects one of the per-stage anchors encoded by bits 12..14 of the
    /// part record's runtime flags.
    pub fn camera_anchor_y(self) -> i32 {
        96 + i32::from((self.0 >> 8) & 0x70)
    }
}

/// Part-record word 13 (`$FF0174`), prepared by `$1B32`, applied by `$1B8E`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoScroll(pub u16);
impl AutoScroll {
    /// Bit 0 set with an axis selects automatic scrolling. While bit 0 is set
    /// the camera is held; `$1BC0` clears it after one frame when bit 1 is set,
    /// otherwise another event must release it.
    pub fn configured(self) -> bool {
        self.0 & 1 != 0 && self.0 & 12 != 0
    }
    pub fn starts_immediately(self) -> bool {
        self.configured() && self.0 & 2 != 0
    }
    pub fn horizontal(self) -> bool {
        self.configured() && self.0 & 4 != 0
    }
    pub fn vertical(self) -> bool {
        self.configured() && self.0 & 8 != 0
    }
    /// 16.16 camera velocity per frame: `(word & $FF00) << 4`, negated by bits 4/5.
    pub fn velocity(self) -> (i32, i32) {
        if !self.configured() {
            return (0, 0);
        }
        let step = i32::from(self.0 & 0xff00) << 4;
        let x = if self.0 & 0x10 != 0 { -step } else { step };
        let y = if self.0 & 0x20 != 0 { -step } else { step };
        (
            if self.horizontal() { x } else { 0 },
            if self.vertical() { y } else { 0 },
        )
    }
}
