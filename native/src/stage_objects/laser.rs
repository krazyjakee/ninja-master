//! Type `$F`, the three 3-2 laser rooms: the list-6 controller (`$2A006`
//! spawn, `$2A116` update) and the type-`$2B` list-1/list-9 objects its
//! spawn allocates, run by `$26178`'s routines:
//!
//! | Routine | Object | Native |
//! | --- | --- | --- |
//! | `$38` x4 | list 1: corner brackets `$10E` sliding out from the screen centre | [`Bracket`] |
//! | `$40` x2 | list 1: edge markers `$10F` on the player's row and column | `Room::markers` |
//! | `$48` | list 1: the reticle `$110..$112`: zooms in, homes on the player at 0.5 px, locks for 128 ticks | [`Reticle`] |
//! | `$50` | list 9: the monster `$113` behind the floor, rising, swaying and opening its jaw `$114..$116` | [`Monster`] |
//! | `$58`, `$08` | list 1: the spray `$117` and the swirl `$365..$368` into the jaw | [`Spark`] |
//! | `$60` x2 | list 9: `$118` with `obj_tile` `$FFFF` at sprite X `+$28` + 8 (9, then 8) and Y `$170`: line masks | [`LaserTables::masks`] |
//!
//! Only the controller hurts: when the lock ends it whitens the palette
//! and gives the player one hit of damage 1 (`obj_flags` bit 14 and
//! `obj_health`, `$2A186`), then fades back over 64 ticks. The monster
//! (`obj_tile` `$2000`) is a low-priority background sprite; nothing it or
//! the other pieces draw collides.
//!
//! Not modelled: the `plr_event` bit 11 the flash sets (`$2A190`) and the
//! `Sys_Flags` bit 1 its end sets (`$2A1CE`).
use super::Sprite;
use crate::{
    combat::PaletteOp,
    enemies::set_whole,
    rom::{Result, Sine, byte, long, signed, word, words},
};

/// Handler constants, each read from the instruction cited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LaserTables {
    /// `$2A056`, `$2A19A`, `$269C0`: effect ids for the entry (and the
    /// monster's retreat), the laser and the monster's lock.
    pub(super) entry_sound: u8,
    flash_sound: u8,
    lock_sound: u8,
    /// `$2A18A`: `move.b #1,7(a3)`, the laser's damage.
    damage: u8,
    /// `$2A166`: line 0's first eleven colours go black; `$2A176`: the
    /// other three lines white.
    black: u8,
    white: u16,
    /// `$2A1B6`: the flash lasts 64 ticks; `$2A1E2`: it fades every fourth.
    flash_ticks: u8,
    fade_mask: u8,
    /// `$265D6`, `Data_0265B6` (tile, 8.8 X and Y speed) and `$26612`.
    bracket_mapping: u16,
    brackets: [(u16, i32, i32); 4],
    bracket_ticks: i32,
    /// `$266AA`/`$266BA` and `$2677C`/`$26788`: the screen centre.
    centre: (i32, i32),
    /// `$266C4`, `$266D2`, `$266F8`: mapping, tile and the edge inset step.
    marker_mapping: u16,
    marker_tile: u16,
    marker_inset: i32,
    /// `$26772`, `Data_0267B4`, `$2687C`: the reticle's tile, zoom poses
    /// and the palette bits the lock toggles.
    reticle_tile: u16,
    reticle_poses: [u16; 4],
    reticle_blink: u16,
    /// `$267EC..$26812`: the screen box the homing reticle stays in.
    clamp_x: (i32, i32),
    clamp_y: (i32, i32),
    /// `$26838`: it aims 32 px above the feet; `$26850`: locks within 4 px.
    aim: i32,
    lock: i32,
    /// `$268B8`, `$268BE`, `$268EC`, `Data_026A28`: the monster, its tile,
    /// the jaw and the jaw frames by lock level.
    monster_mapping: u16,
    monster_tile: u16,
    jaw_mapping: u16,
    jaw_frames: [u16; 4],
    /// `$26974`/`$26984`: the monster's screen anchor; `$26994`: its rise.
    anchor: (i32, i32),
    rise: i32,
    /// `$26B30`, `$26B36`, `$26B50`, `$26B74`, `$26BA0`: the spray's
    /// mapping, tile, screen row, gravity and the band it lives in.
    spray_mapping: u16,
    spray_tile: u16,
    spray_row: i32,
    spray_gravity: i32,
    spray_band: i32,
    /// `$262C0`, `$262C6`, `$262CC`, `$26A8A`: the swirl's radius, life,
    /// first mapping and Y offset.
    swirl_radius: i32,
    swirl_life: i32,
    swirl_mapping: u16,
    swirl_dy: i32,
    /// `$26BAE`, `$26BB4`, `$26BDC`: the masks' mapping, tile word and
    /// sprite Y; `$2A06E`/`$2A08A`: their `+$28` (sprite X - 8,
    /// `$26BD6`), in allocation order.
    pub(super) mask_mapping: u16,
    pub(super) mask_tile: u16,
    mask_y: i32,
    mask_x: [i32; 2],
}
impl LaserTables {
    /// The two `$60` objects' screen origins, in list order (both are in
    /// list 9 before the monster). Their tile word `$FFFF` mirrors both
    /// axes and keeps sprite X 0 (`$14E0`): the first record's pieces sit
    /// at X 1, the second's at X 0 on lines 176..239, so they hide every
    /// later sprite on those lines, the monster's part below the floor
    /// among them (Genesis Plus GX `render_obj_m5`).
    pub(super) fn masks(&self) -> [(i32, i32); 2] {
        self.mask_x.map(|x| (x + 8 - 128, self.mask_y - 128))
    }
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut brackets = [(0, 0, 0); 4];
        for (i, bracket) in brackets.iter_mut().enumerate() {
            let at = 0x265b6 + i * 8;
            *bracket = (word(rom, at)?, signed(rom, at + 2)?, signed(rom, at + 4)?);
        }
        Ok(Self {
            entry_sound: byte(rom, 0x2a057)?,
            flash_sound: byte(rom, 0x2a19b)?,
            lock_sound: byte(rom, 0x269c1)?,
            damage: byte(rom, 0x2a18d)?,
            black: byte(rom, 0x2a167)? + 1,
            white: word(rom, 0x2a178)?,
            flash_ticks: byte(rom, 0x2a1b9)?,
            fade_mask: byte(rom, 0x2a1e5)?,
            bracket_mapping: word(rom, 0x265d8)?,
            brackets,
            bracket_ticks: signed(rom, 0x26614)?,
            centre: (signed(rom, 0x2677e)?, signed(rom, 0x2678a)?),
            marker_mapping: word(rom, 0x266c6)?,
            marker_tile: word(rom, 0x266d4)?,
            marker_inset: signed(rom, 0x266fa)?,
            reticle_tile: word(rom, 0x26774)?,
            reticle_poses: words(rom, 0x267b4)?,
            reticle_blink: word(rom, 0x2687e)?,
            clamp_x: (signed(rom, 0x267ee)?, signed(rom, 0x267f6)?),
            clamp_y: (signed(rom, 0x2680a)?, signed(rom, 0x26812)?),
            aim: signed(rom, 0x2683a)?,
            lock: signed(rom, 0x26852)?,
            monster_mapping: word(rom, 0x268ba)?,
            monster_tile: word(rom, 0x268c0)?,
            jaw_mapping: word(rom, 0x268ee)?,
            jaw_frames: words(rom, 0x26a28)?,
            anchor: (signed(rom, 0x26976)?, signed(rom, 0x26986)?),
            rise: signed(rom, 0x26996)?,
            spray_mapping: word(rom, 0x26b32)?,
            spray_tile: word(rom, 0x26b38)?,
            spray_row: signed(rom, 0x26b52)?,
            spray_gravity: long(rom, 0x26b76)?,
            spray_band: signed(rom, 0x26ba2)?,
            swirl_radius: signed(rom, 0x262c2)?,
            swirl_life: signed(rom, 0x262c8)?,
            swirl_mapping: word(rom, 0x262ce)?,
            swirl_dy: signed(rom, 0x26a8e)?,
            mask_mapping: word(rom, 0x26bb0)?,
            mask_tile: word(rom, 0x26bb6)?,
            mask_y: signed(rom, 0x26bde)?,
            mask_x: [signed(rom, 0x2a070)?, signed(rom, 0x2a08c)?],
        })
    }
}

/// What the room reads each tick.
pub(super) struct Inputs<'a> {
    pub(super) tables: &'a LaserTables,
    pub(super) sine: &'a Sine,
    /// The player's `obj_x`/`obj_y` (feet).
    pub(super) player: (i32, i32),
    pub(super) camera: (i32, i32),
    /// The record's X and Y window (`+$2C..+$32`).
    pub(super) window: ((i32, i32), (i32, i32)),
    /// `Frame_Counter` stand-in.
    pub(super) frame: u64,
    /// `$FF001C`, read without advancing it.
    pub(super) random: u16,
}

/// What one tick requested.
#[derive(Default)]
pub(super) struct Outputs {
    pub(super) sounds: Vec<u8>,
    pub(super) palette_ops: Vec<PaletteOp>,
    /// Damage for `obj_health` with `obj_flags` bit 14 (the laser).
    pub(super) damage: u8,
    /// `Func_027160`: the controller freed itself.
    pub(super) done: bool,
}

/// The controller's `+$36` routines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Routine {
    /// `$2A12C`: wait for the reticle's signal while the player stays in.
    Watch,
    /// `$2A1AE`: the laser's white flash, fading back.
    Flash,
    /// `$2A1FC`: `obj_frame` cleared, so the pieces leave; freed once the
    /// monster reports it has gone.
    Leave,
}

/// One live laser room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Room {
    routine: Routine,
    /// `+$37`: flash ticks.
    count: u8,
    /// `+$38`: 1..4 while the reticle is locked, -1 when it fires, -2 once
    /// the monster has left.
    signal: i16,
    /// `obj_frame` is zero: the pieces test `tst.w $A(a1)`.
    leaving: bool,
    /// A direct stage load has no earlier enemy RNG calls. Advance a local
    /// r * 5 + 1 fallback and a wait-frame rotation while the shared word is
    /// zero, so the visual read can vary without changing the enemy generator.
    charge_random: u16,
    reticle: Option<Reticle>,
    monster: Option<Monster>,
    brackets: Vec<Bracket>,
    /// Routine `$40`: (x, y, tile) of both markers.
    markers: Option<[(i32, i32, u16); 2]>,
    sparks: Vec<Spark>,
}

/// Routine `$48`: `+$36` is the step and `+$37` its counter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Reticle {
    step: u8,
    count: u8,
    /// 16.16 `obj_x`/`obj_y`.
    x: i32,
    y: i32,
    tile: u16,
    mapping: u16,
    /// `obj_flags` bit 11.
    hidden: bool,
    /// Routine `$48` (`$2672A`) ran: the spawn tick only hides it.
    started: bool,
}

/// Routine `$50` and the jaw it allocates (routine -2, no handler).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Monster {
    step: u8,
    count: u8,
    /// `+$28`/`+$2A`: offset from the anchor; `+$2C`: the sway angle.
    dx: i32,
    dy: i32,
    angle: u16,
    x: i32,
    y: i32,
    hidden: bool,
    /// `$2696C` has placed it at least once.
    placed: bool,
    /// `$2690E`: the jaw copies the monster's position before this tick's
    /// move (a recycled record's stale one on the spawn tick: none).
    jaw: Option<(i32, i32)>,
    jaw_mapping: u16,
}

/// Routine `$38`: `+$38` step, `+$3A` counter, `+$30/$34` 16.16 offset from
/// the screen centre.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Bracket {
    tile: u16,
    velocity: (i32, i32),
    offset: (i32, i32),
    step: u8,
    count: i32,
}

/// Routines `$58` and `$08` (`+$40` is the monster).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Spark {
    /// `$26B30`: thrown up from the screen's lower edge under gravity.
    Spray {
        /// The `$FF001C` word and the `+$30` speed word (-2 or -1).
        random: u16,
        speed: i16,
        started: bool,
        /// `+$28`: 16.16 X offset from the monster, `+$2C` its drift.
        offset: i32,
        drift: i32,
        x: i32,
        y: i32,
        vy: i32,
    },
    /// `$262B6`: circles into the jaw on a shrinking radius.
    Swirl {
        angle: u16,
        speed: u16,
        radius: i32,
        life: i32,
        started: bool,
        x: i32,
        y: i32,
        /// `$A`: `$365` + `+$30 >> 3`, taken before the life counts down.
        mapping: u16,
    },
}

impl Room {
    /// `$2A062..$2A110`: the pieces, which run in the spawn tick.
    pub(super) fn new(tables: &LaserTables) -> Self {
        Self {
            routine: Routine::Watch,
            count: 0,
            signal: 0,
            leaving: false,
            charge_random: 0,
            reticle: Some(Reticle {
                step: 0,
                count: 0,
                x: 0,
                y: 0,
                tile: 0,
                mapping: 0,
                // $2672A: hidden until its first zoom step.
                hidden: true,
                started: false,
            }),
            monster: None,
            // Allocated with +$28 = 3, 2, 1, 0.
            brackets: (0..4)
                .rev()
                .map(|i| {
                    let (tile, vx, vy) = tables.brackets[i];
                    Bracket {
                        tile,
                        velocity: (vx << 8, vy << 8),
                        offset: (0, 0),
                        step: 0,
                        count: tables.bracket_ticks,
                    }
                })
                .collect(),
            markers: None,
            sparks: Vec::new(),
        }
    }

    /// One tick: the controller (list 6), then the list-1 pieces, then the
    /// list-9 monster, then the sprite pass's `Object_Integrate`.
    pub(super) fn update(&mut self, input: &Inputs) -> Outputs {
        let mut out = Outputs::default();
        let t = input.tables;
        self.control(input, &mut out);
        if out.done {
            return out;
        }
        if input.random == 0 {
            crate::enemies::next_random(&mut self.charge_random);
            self.charge_random = self.charge_random.rotate_left(7);
        } else {
            self.charge_random = input.random;
        }
        self.update_reticle(input);
        self.update_markers(input);
        let leaving = self.leaving;
        self.brackets.retain_mut(|b| b.update(t, leaving));
        let parent = self.monster.as_ref().map_or((0, 0), |m| (m.x, m.y));
        self.sparks.retain_mut(|s| s.update(input, parent));
        self.update_monster(input, &mut out);
        for spark in &mut self.sparks {
            if let Spark::Spray { y, vy, .. } = spark {
                *vy += t.spray_gravity;
                *y += *vy;
            }
        }
        out
    }

    /// `$2A116`.
    fn control(&mut self, input: &Inputs, out: &mut Outputs) {
        let t = input.tables;
        match self.routine {
            Routine::Watch => {
                if !super::inside(input.window, input.player) {
                    self.routine = Routine::Leave;
                    return;
                }
                if self.signal >= 0 {
                    return;
                }
                // $2A15A: line 0's first colours black, the rest white.
                out.palette_ops.push(PaletteOp::FillFirst(0, t.black, 0));
                for line in 1..4 {
                    out.palette_ops.push(PaletteOp::Fill(line, t.white));
                }
                out.damage = t.damage;
                out.sounds.push(t.flash_sound);
                self.routine = Routine::Flash;
                self.count = 0;
            }
            Routine::Flash => {
                self.count += 1;
                let restore = (0..4).map(PaletteOp::Restore);
                if self.count == t.flash_ticks {
                    // $2A1BC copies the saved palette back; after 15 fades
                    // one more step reaches it.
                    out.palette_ops.extend(restore);
                    self.routine = Routine::Watch;
                    self.count = 0;
                    self.signal = 0;
                } else if self.count & t.fade_mask == 0 {
                    out.palette_ops.extend(restore);
                }
            }
            Routine::Leave => {
                self.leaving = true;
                out.done = self.signal == -2;
            }
        }
    }

    /// Routine `$4C` (`$2673C`).
    fn update_reticle(&mut self, input: &Inputs) {
        let t = input.tables;
        let Some(r) = self.reticle.as_mut() else {
            return;
        };
        if !r.started {
            // $2672A: the spawn tick hides it and clears +$36, nothing more.
            r.started = true;
            return;
        }
        if r.step != 0x10 && self.leaving {
            r.step = 0x10;
            r.count = 0;
        }
        let pose = |count: u8| usize::from(count >> 5) & 3;
        let (cx, cy) = (input.camera.0 + t.centre.0, input.camera.1 + t.centre.1);
        let aim = (input.player.0, input.player.1 - t.aim);
        match r.step {
            0 => {
                // $26772: zoom in at the screen centre.
                r.tile = t.reticle_tile;
                r.x = set_whole(r.x, cx);
                r.y = set_whole(r.y, cy);
                if (r.count as i8) < 0 {
                    r.step = 4;
                    r.count = 0;
                    return;
                }
                r.mapping = t.reticle_poses[pose(r.count)];
                r.count = r.count.wrapping_add(8);
                r.hidden = false;
            }
            4 => {
                // $267E4: stay on screen and home at 0.5 px per axis.
                let (old_x, old_y) = (r.x >> 16, r.y >> 16);
                let (left, right) = (input.camera.0 + t.clamp_x.0, t.clamp_x.1);
                if old_x < left {
                    r.x = set_whole(r.x, left);
                } else if old_x >= left + right {
                    r.x = set_whole(r.x, left + right);
                }
                let (top, bottom) = (input.camera.1 + t.clamp_y.0, t.clamp_y.1);
                if old_y < top {
                    r.y = set_whole(r.y, top);
                } else if old_y >= top + bottom {
                    r.y = set_whole(r.y, top + bottom);
                }
                let (dx, dy) = (aim.0 - old_x, aim.1 - old_y);
                r.x += dx.signum() * 0x8000;
                r.y += dy.signum() * 0x8000;
                if dx.abs() <= t.lock && dy.abs() <= t.lock {
                    self.signal = 1;
                    r.step = 8;
                    r.count = 0;
                }
            }
            8 => {
                // $2686A: locked on, blinking between palettes 1 and 2.
                r.x = set_whole(r.x, aim.0);
                r.y = set_whole(r.y, aim.1);
                r.tile ^= t.reticle_blink;
                if (r.count as i8) < 0 {
                    self.signal = -1;
                    r.step = 0xc;
                    r.hidden = true;
                    return;
                }
                self.signal = (r.count >> 5 & 3) as i16 + 1;
                r.count += 1;
            }
            0xc => {
                // $268AC: wait for the flash to end.
                if self.signal == 0 {
                    r.step = 0;
                    r.count = 0;
                }
            }
            _ => {
                // $267BC: zoom out, then free.
                r.tile = t.reticle_tile;
                if (r.count as i8) < 0 {
                    self.reticle = None;
                    return;
                }
                r.mapping = t.reticle_poses[pose(r.count) ^ 3];
                r.count = r.count.wrapping_add(8);
            }
        }
    }

    /// Routine `$44` (`$266E2`): the left-edge marker on the player's row
    /// and the top-edge marker on its column; freed when the room leaves.
    fn update_markers(&mut self, input: &Inputs) {
        let t = input.tables;
        if self.leaving {
            self.markers = None;
            return;
        }
        let (px, py) = input.player;
        let (cx, cy) = input.camera;
        self.markers = Some([
            (cx + t.marker_inset, py - t.aim, t.marker_tile + 1),
            (px, cy + t.marker_inset * 2, t.marker_tile),
        ]);
    }

    /// Routines `$50`/`$54` (`$268B8`, `$2690E`).
    fn update_monster(&mut self, input: &Inputs, out: &mut Outputs) {
        let t = input.tables;
        // Routine $50 ($268B8) on the spawn tick.
        let m = self.monster.get_or_insert(Monster {
            step: 0,
            count: 0,
            dx: 0,
            dy: 0,
            angle: 0,
            x: 0,
            y: 0,
            hidden: false,
            placed: false,
            jaw: None,
            jaw_mapping: t.jaw_mapping,
        });
        if m.placed {
            m.jaw = Some((m.x, m.y));
        }
        if m.step != 0xc && self.leaving {
            m.step = 0xc;
            m.count = 0;
            out.sounds.push(t.entry_sound);
        }
        let mut spray = None;
        match m.step {
            0 => {
                // $2698E: rise 64 px, jittering and spraying.
                let rest = i32::from(m.count) - t.rise;
                if rest >= 0 {
                    m.step = 4;
                    m.count = 0;
                } else {
                    m.dy = -rest;
                    m.count += 1;
                    m.dx ^= 2;
                    spray = (input.frame & 3 == 0).then_some(-2);
                }
            }
            4 => {
                // $269B0: sway until the reticle locks.
                if self.signal > 0 {
                    m.step = 8;
                    m.count = 0;
                    out.sounds.push(t.lock_sound);
                } else {
                    let sin = |angle: u16| input.sine.polar(angle).1;
                    m.dx = i32::from(((sin(m.angle) << 5) >> 16) as i16);
                    m.dy = i32::from(((sin(m.angle.wrapping_mul(2)) << 2) >> 16) as i16);
                    m.angle = m.angle.wrapping_add(2);
                    spray = (input.frame & 7 == 0).then_some(-1);
                }
            }
            8 => {
                // $26A30: the jaw opens with the lock level and draws the swirl.
                if let Some(jaw) = m.jaw.as_mut() {
                    jaw.1 += 8;
                }
                m.jaw_mapping = t.jaw_mapping;
                if self.signal < 0 {
                    m.step = 4;
                    m.count = 0;
                } else {
                    let level = usize::from(self.signal as u16);
                    if input.frame & 1 != 0
                        && let Some(&frame) = t.jaw_frames.get(level.wrapping_sub(1))
                    {
                        m.jaw_mapping = frame;
                    }
                    if level <= 2 && input.frame & 7 == 0 {
                        // $26A92: the same visual RNG read as Boss2's
                        // $2B6C0 charge, with one emission every 8 frames.
                        let random =
                            crate::ninjutsu_fx::orbit_random(self.charge_random, input.frame >> 3);
                        self.sparks.push(Spark::Swirl {
                            angle: random >> 4,
                            speed: random & 0xf,
                            radius: 0,
                            life: 0,
                            started: false,
                            x: 0,
                            y: 0,
                            mapping: 0,
                        });
                    }
                }
            }
            _ => {
                // $26AB0: sink 64 px, hide for 64 ticks, then report and free.
                m.jaw_mapping = t.jaw_mapping;
                if (m.count as i8) < 0 {
                    self.signal = -2;
                    self.monster = None;
                    return;
                }
                let old = m.count;
                m.count += 1;
                if i32::from(old) >= t.rise {
                    m.hidden = true;
                } else {
                    m.dy += 1;
                    m.dx ^= 2;
                    spray = (input.frame & 3 == 0).then_some(-2);
                }
            }
        }
        if let Some(speed) = spray {
            self.sparks.push(Spark::Spray {
                random: input.random,
                speed,
                started: false,
                offset: 0,
                drift: 0,
                x: 0,
                y: 0,
                vy: 0,
            });
        }
        // $2696C
        m.x = m.dx + input.camera.0 + t.anchor.0;
        m.y = m.dy + input.camera.1 + t.anchor.1;
        m.placed = true;
    }

    /// List 9 in link order: the two `$118` masks (screen positions, as
    /// `$FFFF` records keep them), the monster, then its jaw.
    pub(super) fn back_sprites(&self, t: &LaserTables, camera: (i32, i32)) -> Vec<Sprite> {
        let mut out: Vec<Sprite> = t
            .masks()
            .into_iter()
            .map(|(x, y)| {
                Sprite::new(t.mask_mapping, x + camera.0, y + camera.1)
                    .tiled(t.mask_tile)
                    .flipped(true)
            })
            .collect();
        if let Some(m) = &self.monster {
            if !m.hidden {
                out.push(Sprite::new(t.monster_mapping, m.x, m.y).tiled(t.monster_tile));
            }
            if let Some((x, y)) = m.jaw {
                out.push(Sprite::new(m.jaw_mapping, x, y).tiled(t.monster_tile));
            }
        }
        out
    }
    /// List 1 in link order (Genesis Plus GX, 3-2 tape frame 400: the
    /// order the records were allocated in): the low-priority reticle, the
    /// high-priority markers and brackets (`$A000`, marked high so the
    /// plane's priority cells stay behind them), then the low-priority
    /// sparks.
    pub(super) fn front_sprites(&self, t: &LaserTables, camera: (i32, i32)) -> Vec<Sprite> {
        let mut out = Vec::new();
        if let Some(r) = self.reticle.as_ref().filter(|r| !r.hidden) {
            out.push(Sprite::new(r.mapping, r.x >> 16, r.y >> 16).tiled(r.tile));
        }
        for &(x, y, tile) in self.markers.iter().flatten() {
            out.push(
                Sprite::new(t.marker_mapping, x, y)
                    .tiled(tile)
                    .prioritised(true),
            );
        }
        for b in &self.brackets {
            let x = camera.0 + t.centre.0 + (b.offset.0 >> 16);
            let y = camera.1 + t.centre.1 + (b.offset.1 >> 16);
            out.push(
                Sprite::new(t.bracket_mapping, x, y)
                    .tiled(b.tile)
                    .prioritised(true),
            );
        }
        for spark in &self.sparks {
            match *spark {
                Spark::Spray {
                    started: true,
                    x,
                    y,
                    ..
                } => out.push(Sprite::new(t.spray_mapping, x >> 16, y >> 16).tiled(t.spray_tile)),
                Spark::Swirl {
                    started: true,
                    mapping,
                    x,
                    y,
                    ..
                } => out.push(Sprite::new(mapping, x, y)),
                _ => {}
            }
        }
        out
    }
}

impl Monster {}

impl Bracket {
    /// `$2661C`; false once freed.
    fn update(&mut self, t: &LaserTables, leaving: bool) -> bool {
        match self.step {
            0 => {
                if leaving {
                    return false;
                }
                self.count -= 1;
                if self.count == 0 {
                    self.step = 4;
                } else {
                    self.offset.0 += self.velocity.0;
                    self.offset.1 += self.velocity.1;
                }
            }
            4 => {
                if leaving {
                    self.step = 8;
                    self.count = t.bracket_ticks;
                }
            }
            _ => {
                self.count -= 1;
                if self.count == 0 {
                    return false;
                }
                self.offset.0 -= self.velocity.0;
                self.offset.1 -= self.velocity.1;
            }
        }
        true
    }
}

impl Spark {
    /// One handler call; false once freed. `parent` is the monster's
    /// position (a freed record reads as 0,0).
    fn update(&mut self, input: &Inputs, parent: (i32, i32)) -> bool {
        let t = input.tables;
        match self {
            Spark::Spray {
                random,
                speed,
                started,
                offset,
                drift,
                x,
                y,
                vy,
            } => {
                if !*started {
                    // $26B30
                    *started = true;
                    let r = *random;
                    *offset = i32::from((r as u8 as i8) >> 2) << 16;
                    *y = (input.camera.1 + t.spray_row) << 16;
                    *drift = i32::from(r as i16) << 2;
                    let low = u32::from(r << 4);
                    *vy = (((i32::from(*speed) << 16) as u32 | low) << 1) as i32;
                }
                // $26B80
                *offset += *drift;
                *x = *offset + (parent.0 << 16);
                (((*y >> 16) - input.camera.1) as u16) < t.spray_band as u16
            }
            Spark::Swirl {
                angle,
                speed,
                radius,
                life,
                started,
                x,
                y,
                mapping,
            } => {
                if !*started {
                    // $262B6
                    *started = true;
                    *radius = t.swirl_radius;
                    *life = t.swirl_life;
                }
                // $262D6
                let (ox, oy) = crate::ninjutsu_fx::orbit(input.sine, i32::from(*angle), *radius);
                *x = parent.0 + ox;
                *y = parent.1 + oy + t.swirl_dy;
                *mapping = t.swirl_mapping + (*life >> 3) as u16;
                *angle = angle.wrapping_add(*speed);
                *radius -= 1;
                *life -= 1;
                *life != 0
            }
        }
    }
}
