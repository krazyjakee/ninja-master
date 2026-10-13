//! The title picture and the options backdrop (#64): `Title_Init`,
//! `Title_LoadSprites`, the palette animation `Func_007B74` and the
//! backdrop part of `Options_Init`, repeated as VRAM, CRAM and sprite table

use crate::{
    assets::{kosinski, musashi},
    front_vdp::{FONT_ART, FONT_MAP, map_header, one_bpp, palette_set, tiles},
    render::fade_step,
    rom::{Result, byte, pointer, word, words},
    vdp::{PLANE_ROW_BYTES, Vdp},
};

/// `$80000`: the table of art tables. Entry 0 of its first table is the
/// title's small art (tiles `$40..$69`, the menu cursor among them); entry
/// `$C` of the table at `+$24` is the Musashi picture (Kosinski).
const ART_TABLES: usize = 0x80000;
const PICTURE_ART: (usize, usize) = (0x24, 0x0c);
/// `Title_Init` loads the entry 0 art at tile `$40`.
const SMALL_ART_TILE: u16 = 0x40;
/// The title font is expanded with colour 0 behind `$B`.
const FONT_COLOR: u8 = 0x0b;
/// Pointers to the title data: the logo art (LZ), the logo's two byte maps
/// on plane A, the sprite list, the 1bpp kanji / "III" art and the 1bpp
/// logo outline, and the options' shadow boxes.
const LOGO_ART: usize = 0x3e1ac;
const LOGO_MAPS: [usize; 2] = [0x3e1b0, 0x3e1b4];
const SPRITE_LIST: usize = 0x3e1b8;
const KANJI_ART: usize = 0x3e1bc;
const OUTLINE_ART: usize = 0x3e1c0;
const SHADOW_BOXES: usize = 0x3e1c4;
/// `Data_049E3A`: the Musashi word map (base word, width - 1, height - 1,
/// LZ words), drawn on plane B at `$E420` (row 8, column 16).
const PICTURE_MAP: usize = 0x49e3a;
const PICTURE_VRAM: usize = 0xe420;
const TITLE_PALETTE: usize = 2;
const OPTIONS_PALETTE: usize = 4;
/// `Data_0078C4`: six placeholder sprites (Y 0) ahead of the list.
const PLACEHOLDER_SPRITES: usize = 0x78c4;
/// Sprites the list fills: 6..21.
pub const LIST_FIRST: usize = 6;
pub const LIST_COUNT: usize = 15;
/// Title texts (`Func_0040FA` blocks).
pub const PRESS_START: usize = 0x9080;
pub const BLANK: usize = 0x909c;
pub const COPYRIGHT: usize = 0x90b8;
pub const MENU: usize = 0x90f8;
/// `Mode_Title` scrolls plane A 12 pixels right in 8-line strips.
pub const TITLE_SCROLL: i16 = 12;
/// `Func_007B74` tables: the grey the logo lines fade to, the lines set
/// when the kanji appear, and the pulse of CRAM line 1 colour 11.
const LOGO_GREY: usize = 0x7b32;
const KANJI_LINES: usize = 0x7b12;
const PULSE: usize = 0x7c40;
/// Line 2 colour 13, faded toward red while the kanji slide in.
const KANJI_RED: usize = 45;
const KANJI_RED_TARGET: u16 = 0x000e;
const PULSE_COLOR: usize = 27;
/// `Title_Menu`'s cursor sprite: Y `$109` + 16 per item, 2 x 1 cells,
/// tile `$68` with priority, X `$98`.
const TITLE_CURSOR: [u16; 4] = [0x109, 0x0401, 0x8068, 0x98];
/// Native: the menu is drawn this many plane rows below `MENU`'s row 17,
/// clear of the logo, and gains a QUIT item two rows under OPTIONS.
pub const MENU_DROP: usize = 1;
pub const MENU_ITEMS: usize = 3;
const QUIT_ITEM: &[u8] = b" QUIT";
/// `Options_Menu`'s cursor: Y `$A9` + `Data_008A2E[item]`, tile `$68` in
/// line 3, X `$A8`.
const OPTIONS_CURSOR: [u16; 4] = [0xa9, 0x0401, 0xe068, 0xa8];
const OPTIONS_ROWS: usize = 0x8a2e;
/// `Options_Init`: solid colour-15 tiles `$430..$43F` for the shadow boxes.
const SHADOW_TILE: usize = 0x430;
const SHADOW_BOX_COUNT: usize = 36;

/// The title's VDP state and the palette its fades aim at.
#[derive(Clone)]
pub struct TitleArt {
    pub vdp: Vdp,
    /// `$FF16FA`: the palette `$5EA` / `Func_0006E8` fade toward.
    pub target: [u16; 64],
    /// `$FF00F8`: `Func_007B74` calls so far.
    pub counter: u16,
    font_map: [u8; 128],
    grey: [u16; 32],
    kanji_lines: [u16; 16],
    pulse: [u16; 16],
}
impl TitleArt {
    /// `Title_Init` and `Title_LoadSprites`, with CRAM still black.
    pub fn title(rom: &[u8]) -> Result<Self> {
        let mut font_map = [0; 128];
        font_map.copy_from_slice(rom.get(FONT_MAP..FONT_MAP + 128).ok_or("font map")?);
        let mut art = Self {
            vdp: Vdp::default(),
            target: palette_set(rom, TITLE_PALETTE)?,
            counter: 0,
            font_map,
            grey: words(rom, LOGO_GREY)?,
            kanji_lines: words(rom, KANJI_LINES)?,
            pulse: words(rom, PULSE)?,
        };
        let vdp = &mut art.vdp;
        // $8B02: 8-line strips; plane A 12 pixels right.
        vdp.set_strips(&[[TITLE_SCROLL, 0]; 32]);
        let (first, font) = one_bpp(rom, FONT_ART, 0, FONT_COLOR)?;
        vdp.write_tiles(first, &font);
        // $80000 entry 0: a count word pair, then an LZ stream.
        let small = pointer(rom, pointer(rom, ART_TABLES)?)?;
        let (data, _) = musashi(rom, small + 4, 0x10000)?;
        vdp.write_tiles(
            usize::from(SMALL_ART_TILE),
            &tiles(data, word(rom, small + 2)?),
        );
        art.target[0] = 0x0200;
        art.target[47] = 0x0644;
        draw_logo(vdp, rom)?;
        let table = pointer(rom, ART_TABLES + PICTURE_ART.0)?;
        let picture = pointer(rom, table + PICTURE_ART.1)?;
        let (data, _) = kosinski(rom, picture + 4, 0x10000)?;
        vdp.write_tiles(
            usize::from(word(rom, picture)?),
            &tiles(data, word(rom, picture + 2)?),
        );
        art.load_picture(rom, 0x4000)?;
        art.text(rom, PRESS_START)?;
        art.text(rom, COPYRIGHT)?;
        // Title_LoadSprites.
        let sprites = logo_sprites(rom, 0, 0)?;
        art.vdp.sprites[..sprites.len()].copy_from_slice(&sprites);
        Ok(art)
    }

    /// `Options_Init` over the title's VRAM (the Musashi art is not
    /// reloaded): shadow/highlight on, the picture and logo moved to plane
    /// B without priority, a lit window on plane A, the kanji in line 3 and
    /// 36 shadow-operator sprites. CRAM starts black.
    pub fn options(rom: &[u8]) -> Result<Self> {
        let mut art = Self::title(rom)?;
        art.target = palette_set(rom, OPTIONS_PALETTE)?;
        art.target[0] = 0x0400;
        art.target[61] = 0x0006;
        art.target[27] = 0x0666;
        let vdp = &mut art.vdp;
        vdp.cram = [0; 64];
        vdp.shadow_highlight = true;
        let (first, font) = one_bpp(rom, FONT_ART, 0, FONT_COLOR)?;
        vdp.write_tiles(first, &font);
        // addi.w #$E000 to the map's base word wraps it to palette 0.
        art.load_picture(rom, 0xe000)?;
        let vdp = &mut art.vdp;
        let small = pointer(rom, pointer(rom, ART_TABLES)?)?;
        let (data, _) = musashi(rom, small + 4, 0x10000)?;
        vdp.write_tiles(
            usize::from(SMALL_ART_TILE),
            &tiles(data, word(rom, small + 2)?),
        );
        let (first, outline) = one_bpp(rom, pointer(rom, OUTLINE_ART)?, 0x0f, 0x0d)?;
        vdp.write_tiles(first, &outline);
        art.load_logo(rom)?;
        // The logo maps move to plane B, three cells right ($20060000):
        // the first in line 2 at base $280, the second without priority.
        let vdp = &mut art.vdp;
        for (i, &map) in LOGO_MAPS.iter().enumerate() {
            let at = pointer(rom, map)?;
            let (address, base, width, height) = map_header(rom, at)?;
            let base = if i == 0 { 0x4280 } else { base & 0x7fff };
            let cells: Vec<u16> = rom
                .get(at + 10..at + 10 + width * height)
                .ok_or_else(|| format!("logo map {i} truncated"))?
                .iter()
                .map(|&b| u16::from(b))
                .collect();
            vdp.write_map(address + 0x2006, width, base, &cells);
        }
        // Func_00465E with d0 = $F clears the first expanded tile.
        let (first, mut kanji) = one_bpp(rom, pointer(rom, KANJI_ART)?, 0x0f, 0x0d)?;
        kanji[..32].fill(0);
        vdp.write_tiles(first, &kanji);
        vdp.write_tiles(SHADOW_TILE, &[0xff; 16 * 32]);
        // Func_009018: rows 0-15 get priority tile 0 in columns 4-25, the
        // rest of the 40 x 28 screen is cleared.
        for row in 0..28 {
            for col in 0..40 {
                let lit = row < 16 && (4..26).contains(&col);
                vdp.write_word(
                    vdp.plane_a + row * PLANE_ROW_BYTES + col * 2,
                    if lit { 0x8000 } else { 0 },
                );
            }
        }
        // $8B00: one scroll value for the whole screen, cleared.
        vdp.hscroll.fill([0, 0]);
        vdp.vscroll = [0, 0];
        let list = pointer(rom, SPRITE_LIST)?;
        for i in 0..LIST_COUNT {
            let [y, size, tile, x] = words(rom, list + i * 8)?;
            vdp.sprites[LIST_FIRST + i] = [y, size, tile.wrapping_add(0x2000), x.wrapping_sub(12)];
        }
        let boxes = pointer(rom, SHADOW_BOXES)?;
        let first = LIST_FIRST + LIST_COUNT;
        for i in 0..SHADOW_BOX_COUNT {
            let cell = |offset| -> Result<u16> {
                Ok((i16::from(byte(rom, boxes + i * 2 + offset)? as i8) * 8 + 0x80) as u16)
            };
            let link = ((first + 1 + i) & 0xff) as u16;
            vdp.sprites[first + i] = [cell(0)?, 0x0f00 | link, 0x6430, cell(1)?];
        }
        vdp.sprites[first + SHADOW_BOX_COUNT - 1][1] &= 0xff00;
        set_link(&mut vdp.sprites[14], 0x0f);
        set_link(&mut vdp.sprites[20], 0x15);
        Ok(art)
    }

    /// `Data_03E1AC`: the logo art.
    fn load_logo(&mut self, rom: &[u8]) -> Result<()> {
        logo_art(&mut self.vdp, rom)
    }

    /// `Data_049E3A` with `add` added to its base word.
    fn load_picture(&mut self, rom: &[u8], add: u16) -> Result<()> {
        let base = word(rom, PICTURE_MAP)?.wrapping_add(add);
        let width = usize::from(word(rom, PICTURE_MAP + 2)?) + 1;
        let height = usize::from(word(rom, PICTURE_MAP + 4)?) + 1;
        let (data, _) = musashi(rom, PICTURE_MAP + 6, 0x10000)?;
        let cells: Vec<u16> = data
            .as_chunks::<2>()
            .0
            .iter()
            .take(width * height)
            .map(|b| u16::from_be_bytes(*b))
            .collect();
        self.vdp.write_map(PICTURE_VRAM, width, base, &cells);
        Ok(())
    }

    /// `Func_0040FA`: a text block, each byte through the font map plus the
    /// header's attribute.
    pub fn text(&mut self, rom: &[u8], at: usize) -> Result<()> {
        let (address, base, width, height) = map_header(rom, at)?;
        let cells: Vec<u16> = rom
            .get(at + 10..at + 10 + width * height)
            .ok_or_else(|| format!("text ${at:05x} truncated"))?
            .iter()
            .map(|&b| u16::from(self.font_map[usize::from(b & 0x7f)]))
            .collect();
        self.vdp.write_map(address, width, base, &cells);
        Ok(())
    }

    /// `Options_DrawText`: `text` from a VRAM address, font tiles ORed
    /// with `attribute`.
    pub fn write_text(&mut self, address: usize, text: &[u8], attribute: u16) {
        for (i, &b) in text.iter().enumerate() {
            let tile = u16::from(self.font_map[usize::from(b & 0x7f)]);
            self.vdp.write_word(address + i * 2, tile | attribute);
        }
    }

    /// One `Func_0006E8` update of the whole CRAM toward `target`; false
    /// once nothing changed (`Title_FadeIn` then moves on).
    pub fn fade_in(&mut self) -> bool {
        let before = self.vdp.cram;
        for (color, &target) in self.vdp.cram.iter_mut().zip(&self.target) {
            *color = fade_step(*color, target);
        }
        before != self.vdp.cram
    }

    /// `Func_007B74`, run once per frame of `Title_PressStart` and
    /// `Title_Menu`, then `Data_007C2E`'s pulse of line 1 colour 11.
    ///
    /// - calls `$41..$140`: every 16th, lines 1-2 (the SHINOBI logo) fade
    ///   one step toward grey;
    /// - `$141..$160`: the kanji sprites take priority over the planes and
    ///   lines 1-2 are set to `Data_007B12`;
    /// - `$161..$178`: every other call the kanji and "III" sprites move a
    ///   pixel left, the top 17 strips of plane A a pixel right, "III"
    ///   joins the sprite list and line 2 colour 13 fades toward red;
    /// - after that line 2 colour 13 stays red.
    pub fn animate(&mut self, frame_counter: u32) {
        let count = self.counter;
        self.counter = self.counter.wrapping_add(1);
        let vdp = &mut self.vdp;
        match count {
            0..=0x40 => {}
            0x41..=0x140 => {
                if count & 0xf == 0 {
                    for (color, &target) in vdp.cram[16..48].iter_mut().zip(&self.grey) {
                        *color = fade_step(*color, target);
                    }
                }
            }
            0x141..=0x160 => {
                for sprite in &mut vdp.sprites[LIST_FIRST..LIST_FIRST + 9] {
                    sprite[2] |= 0x8000;
                }
                vdp.cram[16..32].copy_from_slice(&self.kanji_lines);
                vdp.cram[32..48].copy_from_slice(&self.kanji_lines);
            }
            0x161..=0x178 => {
                if count & 1 == 0 {
                    vdp.cram[KANJI_RED] = fade_step(vdp.cram[KANJI_RED], KANJI_RED_TARGET);
                    for sprite in &mut vdp.sprites[LIST_FIRST..LIST_FIRST + LIST_COUNT] {
                        sprite[2] |= 0x8000;
                        sprite[3] = sprite[3].wrapping_sub(1);
                    }
                    set_link(&mut vdp.sprites[14], 0x0f);
                    for line in &mut vdp.hscroll[..17 * 8] {
                        line[0] += 1;
                    }
                }
            }
            _ => vdp.cram[KANJI_RED] = KANJI_RED_TARGET,
        }
        vdp.cram[PULSE_COLOR] = self.pulse[(frame_counter as usize & 0x1e) / 2];
    }

    /// `Title_PressStart` on input: PRESS START is blanked and the menu
    /// drawn, `MENU_DROP` rows lower and with the native QUIT row.
    pub fn show_menu(&mut self, rom: &[u8]) -> Result<()> {
        self.text(rom, BLANK)?;
        let (address, base, width, height) = map_header(rom, MENU)?;
        let text = rom
            .get(MENU + 10..MENU + 10 + width * height)
            .ok_or_else(|| format!("text ${MENU:05x} truncated"))?;
        let mut rows: Vec<Vec<u8>> = text.chunks(width).map(<[u8]>::to_vec).collect();
        let mut quit = vec![b' '; width];
        quit[..QUIT_ITEM.len()].copy_from_slice(QUIT_ITEM);
        rows.extend([vec![b' '; width], quit]);
        let address = address + MENU_DROP * PLANE_ROW_BYTES;
        for (i, row) in rows.iter().enumerate() {
            self.write_text(address + i * PLANE_ROW_BYTES, row, base);
        }
        Ok(())
    }

    /// `Title_Menu`: sprite 0 is the cursor.
    pub fn title_cursor(&mut self, item: usize) {
        let [y, size, tile, x] = TITLE_CURSOR;
        let y = y + 8 * MENU_DROP as u16 + 16 * item as u16;
        self.vdp.sprites[0] = [y, size, tile, x];
    }

    /// `Options_Menu`: sprite 0 is the cursor, beside the item's row.
    pub fn options_cursor(&mut self, rom: &[u8], item: usize) {
        let [y, size, tile, x] = OPTIONS_CURSOR;
        let row = u16::from(byte(rom, OPTIONS_ROWS + item).unwrap_or(0));
        self.vdp.sprites[0] = [y + row, size, tile, x];
    }
}

/// `Data_03E1AC`: the logo art, LZ, after its tile and count words.
fn logo_art(vdp: &mut Vdp, rom: &[u8]) -> Result<()> {
    let logo = pointer(rom, LOGO_ART)?;
    let (data, _) = musashi(rom, logo + 4, 0x10000)?;
    vdp.write_tiles(
        usize::from(word(rom, logo)?),
        &tiles(data, word(rom, logo + 2)?),
    );
    Ok(())
}

/// The SHINOBI logo as `Title_Init` and `Intro_S00` both load it: the 1bpp
/// outline (colour `$D`), the logo art, its two byte maps on plane A and
/// the 1bpp kanji / "III" art (colour `$F`).
pub fn draw_logo(vdp: &mut Vdp, rom: &[u8]) -> Result<()> {
    let (first, outline) = one_bpp(rom, pointer(rom, OUTLINE_ART)?, 0, 0x0d)?;
    vdp.write_tiles(first, &outline);
    logo_art(vdp, rom)?;
    for (i, &map) in LOGO_MAPS.iter().enumerate() {
        let at = pointer(rom, map)?;
        let (address, base, width, height) = map_header(rom, at)?;
        let cells: Vec<u16> = rom
            .get(at + 10..at + 10 + width * height)
            .ok_or_else(|| format!("logo map {i} truncated"))?
            .iter()
            .map(|&b| u16::from(b))
            .collect();
        vdp.write_map(address, width, base, &cells);
    }
    let (first, kanji) = one_bpp(rom, pointer(rom, KANJI_ART)?, 0, 0x0f)?;
    vdp.write_tiles(first, &kanji);
    Ok(())
}

/// Sprites 0..21 of the logo screens: `Data_0078C4`'s six placeholders,
/// then `Data_03E1B8`'s kanji and "III" list moved `dy`, `dx` pixels
/// (`Title_LoadSprites` copies it as it is, `Intro_S00` adds `$20`, `$40`).
pub fn logo_sprites(rom: &[u8], dy: u16, dx: u16) -> Result<[[u16; 4]; LIST_FIRST + LIST_COUNT]> {
    let mut sprites = [[0; 4]; LIST_FIRST + LIST_COUNT];
    for (i, sprite) in sprites[..LIST_FIRST].iter_mut().enumerate() {
        *sprite = words(rom, PLACEHOLDER_SPRITES + i * 8)?;
    }
    let list = pointer(rom, SPRITE_LIST)?;
    for i in 0..LIST_COUNT {
        let [y, size, tile, x] = words(rom, list + i * 8)?;
        sprites[LIST_FIRST + i] = [y.wrapping_add(dy), size, tile, x.wrapping_add(dx)];
    }
    Ok(sprites)
}

/// A sprite table entry's link byte.
fn set_link(sprite: &mut [u16; 4], link: u16) {
    sprite[1] = (sprite[1] & 0xff00) | link;
}
