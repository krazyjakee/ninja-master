//! `Sprite_Mappings` (`$43222`) records: collision boxes and drawing.
use super::Enemy;
use crate::{
    render::{Flip, Frame, HwSprite, Lines, SatWriter, SpriteCell, Tile},
    rom::{Result, SPRITE_MAPPING_COUNT, SpritePiece, sprite_mapping},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hitbox {
    pub x: i32,
    pub y: i32,
    pub half_width: i32,
    pub half_height: i32,
}
impl Hitbox {
    /// No collision area (an empty mapping or a missing record).
    pub const NONE: Self = Self {
        x: 0,
        y: 0,
        half_width: 0,
        half_height: 0,
    };
    pub(crate) fn intersects(self, other: Self) -> bool {
        self.half_width > 0
            && self.half_height > 0
            && other.half_width > 0
            && other.half_height > 0
            && self.collides(other)
    }
    /// This box, its centre an offset, placed at (x, y) with the offset
    /// negated on each flipped axis (`Object_LoadHitbox`, `$18F4`).
    pub(crate) fn placed(self, x: i32, y: i32, horizontal: bool, vertical: bool) -> Self {
        Self {
            x: x + if horizontal { -self.x } else { self.x },
            y: y + if vertical { -self.y } else { self.y },
            ..self
        }
    }
    /// `Object_CollideList` (`$1950..$19D2`), which `Object_CollideL3`
    /// runs for every test against the player's attacks: the centres are
    /// closer than the summed half sizes on both axes, strictly. Unlike
    /// [`Self::intersects`] an empty box is a point, so the sword's
    /// mapping-0 frames (hitbox `0,0,0,0`) still hit a box around them.
    pub(crate) fn collides(self, other: Self) -> bool {
        (self.x - other.x).abs() < self.half_width + other.half_width
            && (self.y - other.y).abs() < self.half_height + other.half_height
    }
}
/// One `Sprite_Mappings` (`$43222`) record: hitbox and sprite pieces.
#[derive(Clone)]
pub(crate) struct Mapping {
    pub(crate) bounds: Hitbox,
    pub(crate) pieces: Vec<SpritePiece>,
}
/// Horizontal extent `(first, last)` of the pieces of `frames` about the
/// object's X, mirrored like a `left`-facing object's sprite; it always
/// includes the origin.
pub(crate) fn sprite_span(
    mappings: &BTreeMap<u16, Mapping>,
    frames: &[u16],
    left: bool,
) -> (i32, i32) {
    let mut span = (0, 0);
    for piece in frames
        .iter()
        .filter_map(|id| mappings.get(id))
        .flat_map(|m| &m.pieces)
    {
        let (lo, hi) = (piece.x, piece.x + piece.width as i32 * 8);
        let (lo, hi) = if left { (-hi, -lo) } else { (lo, hi) };
        span = (span.0.min(lo), span.1.max(hi));
    }
    span
}
/// Hitbox of mapping `id` placed at (x, y); none when the id has no record.
pub(crate) fn mapping_hitbox(
    mappings: &BTreeMap<u16, Mapping>,
    id: u16,
    x: i32,
    y: i32,
    left: bool,
) -> Hitbox {
    mappings
        .get(&id)
        .map_or(Hitbox::NONE, |m| m.bounds.placed(x, y, left, false))
}
/// An enemy's collision box: its explicit `body` when set (bosses and
/// handlers that size their own box), otherwise its mapping's hitbox
/// ([`pose_hitbox`]). Either is flipped like `Object_LoadHitbox` flips it.
pub(crate) fn enemy_hitbox(mappings: &BTreeMap<u16, Mapping>, enemy: &Enemy) -> Hitbox {
    match enemy.body {
        Some(body) => body.placed(enemy.x, enemy.y, enemy.facing_left, enemy.vertical_flip()),
        None => pose_hitbox(mappings, enemy),
    }
}
/// `Object_LoadHitbox` (`$18F4`) for an enemy: its mapping's box, the
/// centre offsets negated by the object's own `obj_tile` flip bits (11,
/// the facing, and 12, [`Enemy::vertical_flip`]), not by the pieces'
/// attribute flips.
pub(crate) fn pose_hitbox(mappings: &BTreeMap<u16, Mapping>, enemy: &Enemy) -> Hitbox {
    mappings.get(&enemy.mapping).map_or(Hitbox::NONE, |m| {
        m.bounds
            .placed(enemy.x, enemy.y, enemy.facing_left, enemy.vertical_flip())
    })
}
impl Mapping {
    pub(crate) fn load(rom: &[u8], id: u16) -> Result<Self> {
        // `$14F0` admits `$3BF`, whose "offset" is the first record's word.
        if id >= SPRITE_MAPPING_COUNT {
            return Err("combat mapping outside verified table".into());
        }
        let (hitbox, pieces) = sprite_mapping(rom, id)?;
        Ok(Self {
            bounds: Hitbox {
                x: i32::from(hitbox[0] as i8),
                y: i32::from(hitbox[1] as i8),
                half_width: i32::from(hitbox[2]),
                half_height: i32::from(hitbox[3]),
            },
            pieces,
        })
    }
}

/// Paints mapping `pieces` at screen `origin`: `base` is added to every
/// attribute (pattern index and palette), `left` mirrors the whole sprite
/// and `tile` resolves a pattern index. Each piece is one hardware sprite
/// ([`Frame::sprite`]); the first SAT sprite wins overlaps, so pieces are
/// painted in reverse; patterns advance down each column. `writer` is the
/// routine that writes them into the sprite table: for
/// [`SatWriter::Object`] this is `Object_DrawSprite` (`$14CA`), which
/// leaves out an object whose origin is 256 pixels or more from the screen
/// centre (`Object_UpdateVisibility`, `$17A8`) unless its `obj_tile` is
/// `$FFFF`, and writes that tile word's pieces as they are
/// ([`SatWriter::Exact`]: at X 0 they mask).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_pieces<'a>(
    frame: &mut Frame,
    pieces: &[SpritePiece],
    palettes: &Lines<'_>,
    base: u16,
    origin: (i32, i32),
    left: bool,
    writer: SatWriter,
    tile: impl Fn(usize) -> Option<&'a Tile>,
) {
    let writer = match writer {
        SatWriter::Object if base == 0xffff => SatWriter::Exact,
        SatWriter::Object
            if origin.0 <= frame.view_left() - 96
                || origin.0 >= frame.view_right() + 96
                || (origin.1 - 112).abs() >= 256 =>
        {
            return;
        }
        writer => writer,
    };
    // `Object_DrawSprite` (`$15DA`): a vertically flipped object mirrors
    // each piece about the origin, -(y + height).
    let vflip = base & 0x1000 != 0;
    for piece in pieces.iter().rev() {
        let attr = piece.attr.wrapping_add(base);
        let piece_y = if vflip {
            -(piece.y + piece.height as i32 * 8)
        } else {
            piece.y
        };
        let flip_x = attr & 0x800 != 0;
        let flip_y = attr & 0x1000 != 0;
        let palette = palettes[usize::from((attr >> 13) & 3)];
        let width = piece.width as i32 * 8;
        let sprite = HwSprite {
            x: origin.0 + if left { -piece.x - width } else { piece.x },
            y: origin.1 + piece_y,
            width: piece.width as u8,
            height: piece.height as u8,
            writer,
        };
        let cells = (0..piece.width).flat_map(|col| {
            let tile = &tile;
            (0..piece.height).filter_map(move |row| {
                let tile = tile(usize::from(attr & 0x7ff) + col * piece.height + row)?;
                let x = piece.x + (if flip_x { piece.width - 1 - col } else { col }) as i32 * 8;
                let y = piece_y + (if flip_y { piece.height - 1 - row } else { row }) as i32 * 8;
                Some(SpriteCell {
                    tile,
                    x: origin.0 + if left { -x - 8 } else { x },
                    y: origin.1 + y,
                    flip: Flip {
                        x: flip_x ^ left,
                        y: flip_y,
                    },
                })
            })
        });
        frame.sprite(sprite, palette, cells);
    }
}

/// `Object_UpdateVisibility` (`$17A8`): an object is drawn while its origin
/// is less than 256 pixels from the screen centre (160, 112) on both axes.
pub(crate) fn object_visible((x, y): (i32, i32)) -> bool {
    (x - 160).abs() < 0x100 && (y - 112).abs() < 0x100
}
