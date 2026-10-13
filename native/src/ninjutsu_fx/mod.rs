//! What the four ninjutsu put on screen (issue #56): the effect objects their
//! handlers allocate, the palette writes, Mijin's shake and the Ikazuchi
//! shield blink.
//!
//! The handlers (`crate::ninjutsu`) allocate list-3 objects with
//! `Object_AllocL3` (at most 20 records while a technique runs, `$22F0A`).
//! Mode `$22A96` (and `$229D8` during Kariu's wave) then integrates every
//! list-3 object and runs its `PlayerAttack_Table` (`$249A6`) handler; list 9
//! runs only Kariu's flames. `Effects` ports those handlers per update, keeps
//! the working palette the handlers write (`$FFF000`) and draws the objects
//! from `Sprite_Mappings` with the art the technique loads. Damage stays with

mod art;

use crate::enemies::set_whole;
use crate::ninjutsu::Kind;
use crate::render::{Frame, Lines, Palette, cram_palettes, fade_step};
use crate::rom::Sine;
use art::Art;
pub use art::Shield;

/// `$25A96`: Kariu dragon sway, 16 signed bytes per dragon.
const SWAY: usize = 0x25a96;
/// List 3 while a technique runs: `$22F0A` writes `$13` (20 records).
const LIST3_LIMIT: usize = 20;
/// List 9: `Object_ListParams+6` = 29 (30 records).
const LIST9_LIMIT: usize = 30;
/// `$FF0014`, gravity (`$AE` writes `$3400`).
const GRAVITY: i32 = 0x3400;

/// Effect palettes, motion and mappings loaded from the cartridge.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct EffectTables {
    shake: [i16; 32],
    glow: [u16; 8],
    dark: [u16; 7],
    gray: [u16; 7],
    dragons: [[i32; 4]; 4],
    pieces: [[i32; 6]; 6],
    debris: [[i32; 3]; 8],
    debris_frames: [(u16, u16); 4],
    flame_frames: [u16; 4],
}
impl EffectTables {
    fn load(rom: &[u8]) -> crate::rom::Result<Self> {
        let mut out = Self {
            glow: crate::rom::words(rom, 0x24932)?,
            dark: crate::rom::words(rom, 0x2496a)?,
            gray: crate::rom::words(rom, 0x24978)?,
            flame_frames: crate::rom::words(rom, 0x26c50)?,
            ..Self::default()
        };
        for (i, value) in out.shake.iter_mut().enumerate() {
            *value = crate::rom::word(rom, 0x20802 + i * 2)? as i16;
        }
        for (i, row) in out.dragons.iter_mut().enumerate() {
            *row = crate::rom::words::<4>(rom, 0x25918 + i * 8)?.map(|v| i32::from(v as i16));
        }
        for (i, row) in out.pieces.iter_mut().enumerate() {
            *row = crate::rom::words::<6>(rom, 0x25820 + i * 16)?.map(|v| i32::from(v as i16));
        }
        for (i, row) in out.debris.iter_mut().enumerate() {
            *row = crate::rom::words::<3>(rom, 0x25c60 + i * 8)?.map(|v| i32::from(v as i16));
        }
        for (i, value) in out.debris_frames.iter_mut().enumerate() {
            let [mapping, tile] = crate::rom::words(rom, 0x25ca0 + i * 4)?;
            *value = (mapping, tile);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod table_tests {
    use super::*;

    #[test]
    fn effect_tables_read_signed_words_and_skip_record_padding() {
        let mut rom = vec![0; 0x27000];
        for (at, value) in [
            (0x24932, 0x345_u16),
            (0x25820, 0xfff1),
            (0x2582a, 5),
            (0x25830, 11),
            (0x25c60, 0xfff7),
            (0x25c68, 13),
            (0x26c50, 0x123),
        ] {
            rom[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
        let tables = EffectTables::load(&rom).unwrap();
        assert_eq!(tables.glow[0], 0x345);
        assert_eq!(tables.pieces[0][0], -15);
        assert_eq!(tables.pieces[0][5], 5);
        assert_eq!(tables.pieces[1][0], 11);
        assert_eq!(tables.debris[0][0], -9);
        assert_eq!(tables.debris[1][0], 13);
        assert_eq!(tables.flame_frames[0], 0x123);
        assert!(EffectTables::load(&[]).is_err());
    }
}

/// Where the frozen player and the camera stand (world pixels, Y the feet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub x: i32,
    pub y: i32,
    pub facing_left: bool,
    pub camera: (i32, i32),
}
impl Anchor {
    /// The player's `obj_tile`: bit 11 mirrors the sprite when facing left.
    fn tile(self) -> u16 {
        if self.facing_left { 0x800 } else { 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Object 21 (`$255EA`, orbit `$262D6`): Ikazuchi sparks.
    Spark,
    /// Object 20 (`$25570`): the thunderbolt.
    Bolt,
    /// Object 22 (`$2564A`): a Fushin double.
    Double,
    /// Object 24 (`$2577E`): a Mijin fireball.
    Piece,
    /// Object 23 (`$2575C`): the fireballs' smoke.
    Smoke,
    /// Object 30 (`$25BEC`): Mijin explosion debris.
    Debris,
    /// Object 25 (`$258F6`): a Kariu fire dragon.
    Dragon,
    /// List-9 object 43, routine `$6E` (`$26BE4`): a dragon's flame.
    Flame,
}

/// One effect record. The handler words are named by record offset.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Object {
    role: Role,
    live: bool,
    /// `obj_flags` bit 11.
    hidden: bool,
    /// `obj_routine`, 0 until the handler initialises the record.
    routine: u8,
    /// `obj_frame` (`Sprite_Mappings` index) and `obj_tile`.
    mapping: u16,
    tile: u16,
    /// 16.16 position, velocity and vertical acceleration.
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    ay: i32,
    w28: i32,
    w2a: i32,
    w2c: i32,
    w2e: i32,
    w30: i32,
    w32: i32,
    w38: i32,
    w3a: i32,
}
impl Object {
    fn new(role: Role, w28: i32) -> Self {
        Self {
            role,
            live: true,
            hidden: false,
            routine: 0,
            mapping: 0,
            tile: 0,
            x: 0,
            y: 0,
            vx: 0,
            vy: 0,
            ay: 0,
            w28,
            w2a: 0,
            w2c: 0,
            w2e: 0,
            w30: 0,
            w32: 0,
            w38: 0,
            w3a: 0,
        }
    }
    /// `$19D4`: acceleration into velocity, velocity into position.
    fn integrate(&mut self) {
        self.vy = self.vy.wrapping_add(self.ay);
        self.x = self.x.wrapping_add(self.vx);
        self.y = self.y.wrapping_add(self.vy);
    }
    fn pixel(&self) -> (i32, i32) {
        (self.x >> 16, self.y >> 16)
    }
}

/// The running technique's objects, palette and shake.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    tables: EffectTables,
    started: bool,
    art: Option<Art>,
    sine: Option<Sine>,
    /// List 3, in allocation order.
    objects: Vec<Object>,
    /// List 9 (Kariu's flames).
    flames: Vec<Object>,
    /// `Frame_Counter` stand-in: updates since the technique began.
    frame: u16,
    /// `$FF001C` (`$96EE`).
    random: u16,
    /// `$FFE040`: the palette when the technique began, every fade target.
    base: [[u16; 16]; 4],
    /// `$FFF000` once a handler writes it.
    cram: Option<[[u16; 16]; 4]>,
    /// Mijin's plane A offset (`VScroll_PlaneA` - `$4C`).
    shake: i32,
}

impl Effects {
    /// One update of mode `$22A96` (or `$229D8`): the technique's handler
    /// when it acts at `cursor` (none while Kariu's wave holds the player),
    /// then lists 3 and 9.
    pub fn tick(
        &mut self,
        rom: &[u8],
        kind: Kind,
        cursor: Option<u16>,
        at: Anchor,
        palette: &[[u16; 16]; 4],
    ) {
        if !self.started {
            self.started = true;
            self.tables = EffectTables::load(rom).unwrap_or_default();
            self.base = *palette;
            self.art = Art::load(rom, kind).ok();
            self.sine = Sine::load(rom).ok();
        }
        self.frame = self.frame.wrapping_add(1);
        if let Some(cursor) = cursor {
            self.act(kind, cursor, at);
        }
        self.run(rom, at);
    }
    /// Stage palettes to draw with: the handlers' working palette if written.
    pub fn palettes(&self, words: &[[u16; 16]; 4]) -> [Palette; 4] {
        cram_palettes(self.cram.as_ref().unwrap_or(words))
    }
    /// Pixels to add to the plane A camera Y (Mijin's shake).
    pub fn shake(&self) -> i32 {
        self.shake
    }
    /// Updates since the technique began (`plr_ticks` while it runs).
    pub fn frame(&self) -> u16 {
        self.frame
    }
    /// The effect sprites, below the player (list order: the earlier
    /// record is in front).
    pub fn draw(&self, frame: &mut Frame, palettes: &Lines<'_>, camera: (i32, i32)) {
        let Some(art) = &self.art else {
            return;
        };
        for object in self.flames.iter().rev().chain(self.objects.iter().rev()) {
            if !object.live || object.hidden {
                continue;
            }
            let (x, y) = object.pixel();
            let origin = (x - camera.0, y - camera.1);
            art.draw(frame, palettes, object.mapping, object.tile, origin);
        }
    }

    fn cram(&mut self) -> &mut [[u16; 16]; 4] {
        let base = self.base;
        self.cram.get_or_insert(base)
    }
    /// `$6E8` over colors `range` (0..64) toward `target`.
    fn fade(&mut self, range: std::ops::Range<usize>, target: impl Fn(usize) -> u16) {
        let cram = self.cram();
        for i in range {
            cram[i / 16][i % 16] = fade_step(cram[i / 16][i % 16], target(i));
        }
    }
    /// `Object_AllocL3`: nothing when list 3 is full.
    fn allocate(&mut self, object: Object) -> bool {
        if self.objects.iter().filter(|o| o.live).count() >= LIST3_LIMIT {
            return false;
        }
        self.objects.push(object);
        true
    }

    /// The handler's cursor actions (`$20B28`, `$20994`, `$208F8`, `$206B2`).
    fn act(&mut self, kind: Kind, cursor: u16, at: Anchor) {
        let high = cursor >> 8;
        match kind {
            Kind::Ikazuchi => {
                // $20B48..$20B88: a spark each update of cursor $05xx.
                if high == 5 {
                    let mut spark = Object::new(Role::Spark, i32::from(cursor & 0xe0) * 4);
                    spark.tile = at.tile();
                    self.allocate(spark);
                }
                if cursor == 0x900 {
                    // $20BA4: all 64 colors white.
                    *self.cram() = [[0xeee; 16]; 4];
                } else if (9..=10).contains(&high) {
                    // $20BC6..$20BE8: fade back toward $FFE040.
                    let base = self.base;
                    self.fade(0..64, |i| base[i / 16][i % 16]);
                }
                if cursor == 0xd00 {
                    // $20C12: object 20 with +$3A = $50 ($20C1E).
                    let mut bolt = Object::new(Role::Bolt, 0);
                    bolt.tile = at.tile();
                    bolt.w3a = 0x50;
                    self.allocate(bolt);
                }
            }
            Kind::Kariu => {
                let base = self.base;
                // $21A24/$21A3E: Joe glows (cursor $09xx..$0Axx), then fades
                // back ($0Exx..$0Fxx).
                if (9..=10).contains(&high) {
                    let glow = self.tables.glow;
                    self.fade(1..9, |i| glow[i - 1]);
                }
                if (0xe..=0xf).contains(&high) {
                    self.fade(1..9, |i| base[0][i]);
                }
                if cursor == 0x900 {
                    // $20A1A..$20A40, $20B0C: red +2 from line 1 color 2,
                    // three more times on line 1.
                    let cram = self.cram();
                    for (start, count) in [(18, 46), (18, 14), (18, 14), (18, 14)] {
                        for i in start..start + count {
                            let color = &mut cram[i / 16][i % 16];
                            let red = *color & 0xf;
                            *color = (*color & 0xff0) | (if red < 0xe { red + 2 } else { red });
                        }
                    }
                }
                if cursor == 0xd20 {
                    // $20A44: one step of lines 1..3 toward $FFE060.
                    self.fade(16..64, |i| base[i / 16][i % 16]);
                }
                if cursor == 0xd00 {
                    // $20A6C..$20A82: four object-25 dragons, +$28 = 3..0.
                    for index in (0..4).rev() {
                        let mut dragon = Object::new(Role::Dragon, index);
                        dragon.tile = at.tile();
                        if !self.allocate(dragon) {
                            break;
                        }
                    }
                }
            }
            Kind::Fushin => {
                if cursor == 0x900 {
                    // $2095E..$20976: four object-22 doubles, +$28 = 3..0.
                    for index in (0..4).rev() {
                        let mut double = Object::new(Role::Double, index);
                        double.tile = at.tile();
                        if !self.allocate(double) {
                            break;
                        }
                    }
                }
            }
            Kind::Mijin => {
                // Ninjutsu_MijinBlast ($20842..$20896): line 0 colors 1..7.
                let colors = match cursor {
                    0xe00 | 0x2500 => Some(self.tables.dark),
                    0x1000 => Some(self.tables.gray),
                    0x2700 => {
                        let base = self.base[0];
                        Some(std::array::from_fn(|i| base[i + 1]))
                    }
                    _ => None,
                };
                if let Some(colors) = colors {
                    self.cram()[0][1..8].copy_from_slice(&colors);
                }
                if cursor == 0x1400 {
                    // $208C8..$208F0: six object-24 fireballs, +$28 = 5..0,
                    // then eight object-30 debris, +$28 = 7..0.
                    for (role, count) in [(Role::Piece, 6), (Role::Debris, 8)] {
                        for index in (0..count).rev() {
                            let mut object = Object::new(role, index);
                            object.tile = at.tile();
                            if !self.allocate(object) {
                                return;
                            }
                        }
                    }
                }
                // $207B8..$207FE: every other update until $1C00.
                if (0x1420..0x1c00).contains(&cursor) && (cursor - 0x1420) & 0x20 == 0 {
                    self.shake = -i32::from(self.tables.shake[usize::from((cursor - 0x1420) >> 6)]);
                }
            }
        }
    }

    /// Lists 3 and 9 (`$22A9C..$22AF6`): integrate, then the handler.
    fn run(&mut self, rom: &[u8], at: Anchor) {
        let mut i = 0;
        while i < self.objects.len() {
            if self.objects[i].live {
                let mut object = self.objects[i].clone();
                object.integrate();
                self.update(rom, &mut object, at);
                self.objects[i] = object;
            }
            i += 1;
        }
        self.objects.retain(|o| o.live);
        let mut flames = std::mem::take(&mut self.flames);
        for flame in &mut flames {
            flame.integrate();
            self.update_flame(flame);
        }
        flames.retain(|o| o.live);
        self.flames = flames;
    }

    fn update(&mut self, rom: &[u8], o: &mut Object, at: Anchor) {
        let fc = i32::from(self.frame);
        let run = match o.role {
            Role::Spark => {
                if o.routine == 0 {
                    // $255FA: centre 72 px above Joe, radius and count 31.
                    o.w2c = 0;
                    o.w2e = -72;
                    o.tile &= 0x8000;
                    o.w30 = 0x1f;
                    o.w32 = 0x365;
                    o.w2a = 0x1f;
                    o.routine = 4;
                }
                // $262D6: spiral in, 16 angle steps per update.
                let (dx, dy) = self
                    .sine
                    .as_ref()
                    .map_or((0, 0), |sine| orbit(sine, o.w28, o.w2a));
                o.x = set_whole(o.x, at.x + dx + o.w2c);
                o.y = set_whole(o.y, at.y + dy + o.w2e);
                o.mapping = ((o.w30 >> 3) + o.w32) as u16;
                o.w28 += 0x10;
                o.w2a -= 1;
                o.w30 -= 1;
                if o.w30 == 0 {
                    o.live = false;
                }
                false
            }
            Role::Bolt => {
                // $25580 / $255B4: over Joe's head, alternately mirrored,
                // jittering 0..24 px down with Frame_Counter.
                if o.routine == 0 {
                    o.mapping = 0xe1;
                    o.tile = (o.tile & 0x9800) ^ 0x800;
                    o.routine = 4;
                } else {
                    o.tile ^= 0x800;
                }
                o.x = at.x << 16;
                o.y = set_whole(o.y, at.y + (fc & 3) * 8 - 0xc0);
                true
            }
            Role::Double => {
                o.hidden = !o.hidden;
                self.update_double(o, at);
                true
            }
            Role::Piece => {
                self.update_piece(o, at);
                true
            }
            Role::Smoke => {
                // $2575C: flickers, next mapping every 8 updates.
                o.hidden = !o.hidden;
                o.w28 -= 1;
                if o.w28 == 0 {
                    o.live = false;
                } else if o.w28 & 7 == 0 {
                    o.mapping += 1;
                }
                false
            }
            Role::Debris => {
                if o.routine == 0 {
                    // $25BFC
                    let [vx, vy, gravity] = self.tables.debris[o.w28 as usize & 7];
                    o.w38 = 0x1f;
                    o.w3a = 0x7fff;
                    o.vx = vx << 16;
                    o.vy = vy << 16;
                    o.x = at.x << 16;
                    o.y = at.y << 16;
                    o.ay = if gravity != 0 { GRAVITY } else { 0 };
                    o.w2a = 0;
                    o.routine = 4;
                }
                // $25CC0
                if o.w2a != 0 {
                    o.w2a -= 1;
                } else {
                    let (mapping, tile) = self.tables.debris_frames[(o.w38 as usize & 0x38) >> 3];
                    o.mapping = 0xc9 + mapping;
                    o.tile = (o.tile & 0xf800) | tile;
                    o.w38 -= 1;
                    if o.w38 < 0 {
                        o.live = false;
                    }
                }
                o.live
            }
            Role::Dragon => {
                self.update_dragon(rom, o, at);
                false
            }
            Role::Flame => false,
        };
        // PlayerAttack_Run ($24A80): +$3A counts the record's lifetime.
        if run && o.live {
            o.w3a -= 1;
            if o.w3a == 0 {
                o.live = false;
            }
        }
    }

    /// Object 22: split sideways, wait, leap, then drop back onto Joe.
    fn update_double(&mut self, o: &mut Object, at: Anchor) {
        if o.routine == 0 {
            // $2566A: even +$28 go right, odd left; +$38 delay, +$3A life.
            o.hidden = false;
            o.mapping = 0x49;
            o.x = at.x << 16;
            o.y = at.y << 16;
            o.vx = if o.w28 & 1 == 0 { 2 << 16 } else { -2 << 16 };
            o.w38 = ((o.w28 << 4) & 0x20) + 0x20;
            o.w3a = ((o.w28 << 4) & 0x30) + 0x88;
            o.routine = 4;
        }
        if o.routine == 4 {
            // $256D8
            o.w38 -= 1;
            if o.w38 != 0 {
                return;
            }
            o.vx = 0;
            o.w38 = 0x20;
            o.routine = 8;
        }
        if o.routine == 8 {
            // $256F0: effect 5, leap at 8 px/update as player frame 29.
            o.w38 -= 1;
            if o.w38 != 0 {
                return;
            }
            o.vy = set_whole(o.vy, -8);
            o.mapping = 8;
            o.tile = (o.tile & 0xff00) | 0x1b;
            o.w38 = 0x1c;
            o.routine = 12;
        }
        if o.routine == 12 {
            // $25726
            o.w38 -= 1;
            if o.w38 != 0 {
                return;
            }
            o.vy = 0;
            o.routine = 16;
        }
        // $25738: the last 28 updates fall onto Joe as player frame 28.
        if o.w3a < 0x1c {
            o.mapping = 7;
            o.tile &= 0xff00;
            o.x = at.x << 16;
            o.vy = set_whole(o.vy, 8);
        }
    }

    /// Object 24: fly out for 64 updates trailing smoke, then back, blinking.
    fn update_piece(&mut self, o: &mut Object, at: Anchor) {
        o.tile ^= 0x8000;
        if o.routine == 0 {
            // $257AC
            let [vx, vy, dx, dy, mapping, smoke] = self.tables.pieces[o.w28 as usize % 6];
            o.w3a = 0x80;
            o.vx = vx << 16;
            o.vy = vy << 16;
            o.x = (at.x + dx) << 16;
            o.y = (at.y + dy) << 16;
            o.w2a = smoke;
            o.mapping = (0xc9 + mapping) as u16;
            o.w28 = 0x40;
            o.routine = 4;
        }
        let fc = i32::from(self.frame);
        if o.routine == 4 {
            // $25880: object 23 smoke every eighth frame.
            if o.w2a != 0 && fc & 7 == 0 {
                let mut smoke = Object::new(Role::Smoke, 0x20);
                smoke.tile = o.tile & 0x8000;
                smoke.x = o.x;
                smoke.y = o.y;
                smoke.mapping = 0x35f;
                self.allocate(smoke);
            }
            o.w28 -= 1;
            if o.w28 == 0 {
                o.vx = set_whole(o.vx, -(o.vx >> 16));
                o.vy = set_whole(o.vy, -(o.vy >> 16));
                o.routine = 8;
            }
        } else if fc & o.w2a != 0 {
            // $258E4
            o.hidden = !o.hidden;
        }
    }

    /// Object 25: wait, rise from below the screen, cross it and sway.
    fn update_dragon(&mut self, rom: &[u8], o: &mut Object, at: Anchor) {
        o.tile ^= 0x8000;
        o.w3a += 1;
        let fc = i32::from(self.frame);
        let (camera_x, camera_y) = at.camera;
        if o.routine == 0 {
            // $25938
            let index = o.w28 & 3;
            let [x, delay, wait, speed] = self.tables.dragons[index as usize];
            o.mapping = 0xdd;
            o.w2a = 0;
            o.w30 = 0;
            o.w2c = wait;
            o.w2e = speed;
            o.w28 = delay;
            o.x = set_whole(o.x, camera_x + x);
            o.y = set_whole(o.y, camera_y + 0x160);
            o.w32 = index * 16;
            o.routine = 4;
        }
        if o.routine == 4 {
            // $2599E: after the delay (effect 12), 28 updates up 8 px.
            if o.w28 > 0 {
                o.w28 -= 1;
                return;
            }
            if o.w28 == 0 {
                o.w28 = -1;
            }
            if o.w2a < 28 {
                o.w2a += 1;
                o.y = o.y.wrapping_sub(8 << 16);
                self.flame(o);
                return;
            }
            o.routine = 8;
        }
        self.flame(o);
        o.y = set_whole(o.y, camera_y + 0x60 - (fc & 3) * 8);
        if o.routine == 8 {
            // $259DC: wait, then 12 steps across.
            if o.w2c != 0 {
                o.w2c -= 1;
                return;
            }
            if o.w30 != 12 {
                o.w30 += 1;
                o.x = o.x.wrapping_add(o.w2e << 16);
                return;
            }
            o.w28 = 0x40;
            o.w2a = 0;
            o.routine = 12;
        }
        // $25A2C: sway for 64 updates, then 64 more steps across.
        if o.w28 == 0 {
            if o.w2a == 0x40 {
                o.live = false;
                return;
            }
            o.w2a += 1;
            o.x = o.x.wrapping_add(o.w2e << 16);
            return;
        }
        o.w28 -= 1;
        let sway = rom
            .get(SWAY + (o.w32 + (o.w28 & 15)) as usize)
            .map_or(0, |b| i32::from(*b as i8));
        o.x = o.x.wrapping_add(sway << 16);
    }
    /// `$25AD6`: a list-9 flame every fourth dragon update.
    fn flame(&mut self, dragon: &Object) {
        if dragon.w3a & 3 != 0 || self.flames.len() >= LIST9_LIMIT {
            return;
        }
        let mut flame = Object::new(Role::Flame, 0);
        let (x, y) = dragon.pixel();
        flame.x = x << 16;
        flame.y = y << 16;
        self.flames.push(flame);
    }
    /// `$26BE4`: a random drift up that gravity turns, 31 updates.
    fn update_flame(&mut self, o: &mut Object) {
        if o.routine == 0 {
            let random = crate::enemies::next_random(&mut self.random) as i16;
            o.tile = 0x8000;
            o.vx = ((i32::from(random) << 2) >> 16) << 16;
            o.vy = -4 << 16;
            o.y = o.y.wrapping_add((i32::from(random as u8 as i8) >> 1) << 16);
            o.ay = 0x3400;
            o.w3a = 31;
            o.routine = 4;
        }
        o.mapping = self.tables.flame_frames[(o.w3a as usize >> 3) & 3];
        o.w3a -= 1;
        if o.w3a == 0 {
            o.live = false;
        }
    }
}

/// A visual read of `$FF001C` for a charge particle. `Main_WaitFrame`
/// (`$512`) rotates the word until VBlank; native uses seven turns per
/// emission on a copy, so idle charges vary without changing enemy RNG.
/// The exact rotation count depends on CPU work and cannot be matched.
pub(crate) fn orbit_random(random: u16, emission: u64) -> u16 {
    random.rotate_left((emission.wrapping_mul(7) & 15) as u32)
}

/// `$94A0` and the `$262D6` scaling: offset of radius `radius` at `angle`
/// (`$400` per turn). X is stretched by 17/16.
pub(crate) fn orbit(sine: &Sine, angle: i32, radius: i32) -> (i32, i32) {
    let (cos, sin) = sine.polar(angle as u16);
    let scale = |value: i32| {
        let product = i32::from((value >> 2) as i16) * i32::from(radius as i16);
        i32::from((product.wrapping_shl(2) >> 16) as i16)
    };
    (scale(cos), scale(sin))
}

/// `$2158E..$215C6`: while Ikazuchi charges remain (+$30), every fourth
/// player update shows shield mapping `$62`/`$63` (alternating every four;
/// `$86`/`$87` in states `$20..$2F`) with the `$2454A` art.
pub fn shield_mapping(shield: u8, ticks: u64, state: usize) -> Option<u16> {
    if shield == 0 || !ticks.is_multiple_of(4) {
        return None;
    }
    let mut id = 0x62 + ((ticks & 4) >> 2) as u16;
    if state & !0xf == 0x20 {
        id += 0x24;
    }
    Some(id)
}
