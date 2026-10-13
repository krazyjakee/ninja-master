//! Boss4's effects outside its sprite parts: the gunboat's charge (steam,
//! the palette and plane-A flash, the bow wave).
//!
//! Records allocated during the boss update (list 1, list 9, list 4) have
//! their first update in the same frame (`Game_RunFrame` runs `Boss_Update`
//! before `Func_01CC0A`'s list 4 and `Func_0366A6`'s lists 1 and 9); the
//! effect pass runs after the boss natively too.
use super::Boss4;
use crate::{
    combat::{Effect, EffectKind, Enemy, Hitbox, PaletteOp, PlaneOp, Projectile, ShotBox},
    enemies::Ctx,
    rom::{Result, pointer, signed, word, words},
};

/// The gunboat's charge (routines `$20..$28`, `Func_02DF26`,
/// `Func_02DF86`, `Func_02E106`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Charge {
    /// `$2DFB2`: while `+$28` (after the update's `subq`) is at least
    /// `steam_from`, every update with `+$28 & steam_period` = 0 puffs
    /// steam: a list-1 `$2B` record at routine `$0C` (`$262D6`, the
    /// circling spark of [`EffectKind::Orbit`]).
    pub(super) steam_from: i32,
    pub(super) steam_period: i32,
    /// Its centre from the boss (`+$2C`/`+$2E`; the parent pointer `+$40`
    /// is left 0, whose `+$20`/`+$24` are the high words of vectors 8 and
    /// 9, both 0), the angle `($FF001C & mask) + base` (read without
    /// advancing it), the radius, life and first mapping (`+$2A`, `+$30`,
    /// `+$32`: frame `+$32` + `+$30` >> 3).
    pub(super) steam_offset: (i32, i32),
    pub(super) steam_angle: (u16, u16),
    pub(super) steam_radius: i32,
    pub(super) steam_life: i32,
    pub(super) steam_mapping: u16,
    /// `Data_02E1D2`: CRAM line 3 colours 12..15 when the gunboat reaches
    /// X `$150` and after the charge on odd `+$28` (the first four) or
    /// even (the last four); `Data_02E1E2` when the flicker ends. Colour 1
    /// of line 3 is `$EEE` from the arrival on (`$2DF4E`).
    pub(super) colors: [u16; 8],
    pub(super) settled: [u16; 4],
    /// `$2E054`: line 0's first twelve colours black and lines 1..3 white.
    pub(super) black: u8,
    /// `Func_02F08E` (`Data_00C180`): plane A rows 3..5, columns 0..4 and
    /// 32..63 (left of the hull), one cell pair per row: at the charge's
    /// end, then from `$2E1AA` on every eighth update after it (`+$28`
    /// `$20`, `$18`, `$10`, 8), the third row being the first with
    /// `eori.l #$10001000`.
    pub(super) cells_vram: u16,
    pub(super) end_cells: [u32; 3],
    pub(super) fading_cells: [[u32; 2]; 4],
    pub(super) cells_flip: u32,
    /// List-4 `$34` routine `$20` (`$2FAB0`): the bow wave, with the boss
    /// as its `+$3C`: mapping 0, box `$80007F0C` (never flipped), damage
    /// byte, `+$28` life; each update adds the boss X - `reach` to its X
    /// (0 when allocated) and takes the boss Y, so it sweeps the screen
    /// from the bow in two frames and leaves it.
    pub(super) bow_box: Hitbox,
    pub(super) bow_damage: u8,
    pub(super) bow_life: u16,
    pub(super) bow_reach: i32,
}

fn opcode(rom: &[u8], at: usize, expected: u16, what: &str) -> Result<()> {
    if word(rom, at)? != expected {
        return Err(format!("{what} differs at ${at:05x}"));
    }
    Ok(())
}
fn long(rom: &[u8], at: usize) -> Result<u32> {
    Ok(pointer(rom, at)? as u32)
}

impl Charge {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        const STEAM: &str = "Func_02DF86's steam";
        // cmpi.w #$40,d0 / andi.w #3,d0 / moveq #$2B,d0 / subi.w #$48,d0 /
        // addq.w #4,d0 / andi.w #$FF,d0 / addi.w #$180,d0 / move.w #,$2A /
        // move.w #,$30 / move.w #,$32 / move.w #$C,4(a6).
        for (at, op) in [
            (0x2dfb2, 0x0c40),
            (0x2dfb8, 0x0240),
            (0x2dfbe, 0x702b),
            (0x2dfcc, 0x0440),
            (0x2dfd8, 0x5840),
            (0x2dfe4, 0x0240),
            (0x2dfe8, 0x0640),
            (0x2e004, 0x3d7c),
            (0x2e00a, 0x3d7c),
            (0x2e010, 0x3d7c),
            (0x2e01a, 0x3d7c),
        ] {
            opcode(rom, at, op, STEAM)?;
        }
        if word(rom, 0x2e01c)? != 0x0c || word(rom, 0x2e016)? != 0x3d40 {
            return Err("Func_02DF86's steam is not a routine-$0C orbit".into());
        }
        // move.l #$EEE0EEE,d0 / moveq #$17,d1 / moveq #$B,d1 / addq.w #8,a1.
        opcode(rom, 0x2e06a, 0x720b, "the charge's flash")?;
        opcode(rom, 0x2e072, 0x5049, "the charge's flash")?;
        opcode(rom, 0x2e07a, 0x7217, "the charge's flash")?;
        // lea (Data_00C180).l,a5 in Func_02F08E; eori.l #$10001000,d5.
        opcode(rom, 0x2f094, 0x4bf9, "Func_02F08E")?;
        opcode(rom, 0x2e190, 0x0a85, "Func_02E106's cells")?;
        let bow_box = long(rom, 0x2fac4)?;
        opcode(rom, 0x2fac2, 0x217c, "the bow wave")?;
        opcode(rom, 0x2fae2, 0x0440, "the bow wave")?;
        // $2E1AA + +$28 (8, $10, $18, $20): d3, then d4 four bytes on.
        let fading_cells = [8, 0x10, 0x18, 0x20].map(|t| {
            let at = 0x2e1aa + t;
            [long(rom, at), long(rom, at + 4)]
        });
        let fading_cells = fading_cells.map(|[a, b]| Ok::<_, String>([a?, b?]));
        let [a, b, c, d] = fading_cells;
        Ok(Self {
            steam_from: signed(rom, 0x2dfb4)?,
            steam_period: signed(rom, 0x2dfba)?,
            steam_offset: (-signed(rom, 0x2dfce)?, 4),
            steam_angle: (word(rom, 0x2dfe6)?, word(rom, 0x2dfea)?),
            steam_radius: signed(rom, 0x2e006)?,
            steam_life: signed(rom, 0x2e00c)?,
            steam_mapping: word(rom, 0x2e012)?,
            colors: words(rom, 0x2e1d2)?,
            settled: words(rom, 0x2e1e2)?,
            black: (word(rom, 0x2e06a)? & 0xff) as u8 + 1,
            cells_vram: long(rom, 0x2f096)? as u16,
            end_cells: [
                long(rom, 0x2e084)?,
                long(rom, 0x2e08a)?,
                long(rom, 0x2e090)?,
            ],
            fading_cells: [a?, b?, c?, d?],
            cells_flip: long(rom, 0x2e192)?,
            bow_box: super::hitbox(bow_box),
            bow_damage: (word(rom, 0x2facc)? >> 8) as u8,
            bow_life: word(rom, 0x2fad2)?,
            bow_reach: signed(rom, 0x2fae4)?,
        })
    }
}

/// Line 3's colour 1 and colours 12..15 (`$FFF062`, `$FFF078`).
const LINE: u8 = 3;
const WHITE_INDEX: u8 = 1;
const FIRST_INDEX: u8 = 12;
const WHITE: u16 = 0x0eee;

impl Boss4 {
    fn line3(cx: &mut Ctx, colors: &[u16]) {
        for (i, &color) in colors.iter().enumerate() {
            cx.palette_op(PaletteOp::Color(LINE, FIRST_INDEX + i as u8, color));
        }
    }
    /// `Func_02DF26`: at X `$150` line 3 takes its charge colours (in
    /// CRAM and in the `$FF16FA` fade target alike).
    pub(super) fn charge_colors(&self, cx: &mut Ctx) {
        cx.palette_op(PaletteOp::Color(LINE, WHITE_INDEX, WHITE));
        Self::line3(cx, &self.fx.charge.colors[..4]);
    }
    /// `$2DFB2`: a puff of steam before the bow at `+$28` = `t`.
    pub(super) fn steam(&self, cx: &mut Ctx, e: &Enemy, t: i32) -> bool {
        let c = &self.fx.charge;
        if t < c.steam_from || t & c.steam_period != 0 {
            return false;
        }
        let (mask, base) = c.steam_angle;
        let mut steam = Effect::new(
            EffectKind::Orbit,
            0,
            0,
            c.steam_mapping + (c.steam_life >> 3) as u16,
        );
        steam.angle = (*cx.random & mask).wrapping_add(base);
        steam.counter = c.steam_life;
        steam.origin = (e.x + c.steam_offset.0, e.y + c.steam_offset.1);
        // Placed by its first update, in this frame's effect pass, at the
        // radius `+$2A` = life + 8.
        debug_assert_eq!(c.steam_radius, c.steam_life + 8);
        cx.effect(steam);
        true
    }
    /// `$2E054`: line 0's first colours black and lines 1..3 white, the
    /// plane-A cells beside the bow, and the bow wave.
    pub(super) fn charge_flash(&self, cx: &mut Ctx) {
        let c = &self.fx.charge;
        cx.palette_op(PaletteOp::FillFirst(0, c.black, 0));
        for line in 1..4 {
            cx.fill_palette(line, WHITE);
        }
        self.cells(cx, c.end_cells);
        let mut wave = Projectile::new(0, 0, 0, true, 0, c.bow_damage);
        wave.box_load = ShotBox::Fixed(c.bow_box);
        wave.life = c.bow_life;
        wave.passes_terrain = true;
        // `bclr #6,(a0)`: touching the player does not free it; nor does
        // leaving the view (its second place, X 416, is off screen and
        // its box, 161..415, still meets the player), only its `+$28`.
        wave.stays = true;
        wave.left_edge_only = true;
        wave.owner = Some(cx.current);
        cx.shoot(wave);
    }
    /// `Func_02F08E`: rows 3..5 of plane A, columns 0..4 and 32..63.
    fn cells(&self, cx: &mut Ctx, rows: [u32; 3]) {
        let base = self.fx.charge.cells_vram;
        for (row, cells) in rows.into_iter().enumerate() {
            let vram = base + row as u16 * 0x80;
            // move.l, move.l, move.w: five cells, then sixteen longs $40
            // bytes on. Both words of each long are the same cell.
            for (vram, width) in [(vram, 5), (vram + 0x40, 32)] {
                cx.plane_op(PlaneOp::Fill {
                    vram,
                    width,
                    height: 1,
                    cell: cells as u16,
                });
            }
        }
    }
    /// `Func_02E106` while `+$28` = `t` (after its `subq`) runs: on even
    /// updates the whole palette steps back toward `$FF16FA` (`Func_0006E8`;
    /// line 3's colour 1 is white in the target) and every eighth the cells
    /// fade; line 3's colours 12..15 flicker on every update and settle on
    /// white at 0.
    pub(super) fn charge_fade(&self, cx: &mut Ctx, t: i32) {
        let c = &self.fx.charge;
        if t == 0 {
            Self::line3(cx, &c.settled);
            return;
        }
        if t & 1 == 0 {
            for line in 0..4 {
                cx.restore_palette(line);
            }
            cx.palette_op(PaletteOp::Color(LINE, WHITE_INDEX, WHITE));
            if t & 7 == 0
                && let Some(&[d3, d4]) = c.fading_cells.get((t >> 3) as usize - 1)
            {
                self.cells(cx, [d3, d4, d3 ^ c.cells_flip]);
            }
        }
        let half = if t & 1 != 0 { 0 } else { 4 };
        Self::line3(cx, &c.colors[half..half + 4]);
    }
    /// List-4 routine `$24` (`$2FADA`) for the bow waves: X += boss X -
    /// `$80`, Y = boss Y, after the boss moved.
    pub(super) fn sweep_bow_waves(&self, cx: &mut Ctx, e: &Enemy) {
        let c = &self.fx.charge;
        let me = cx.current;
        for shot in cx.projectiles.iter_mut() {
            if shot.owner == Some(me) && shot.alive && shot.box_load == ShotBox::Fixed(c.bow_box) {
                shot.x += e.x - c.bow_reach;
                shot.y = e.y;
            }
        }
    }
}

/// The effect tables of [`Boss4`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Effects {
    pub(super) charge: Charge,
}
impl Effects {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            charge: Charge::load(rom)?,
        })
    }
}
