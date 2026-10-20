use super::{MAX_PACK_BYTES, MAX_SCALE, Palette, Replacement, Source, image};
use crate::{assets::PlayerAssets, render::Flip};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// An independent player pose; coordinates are relative to the original feet
/// origin. Unlike tile edits, different poses may repaint shared ROM patterns.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSheet {
    pub file: String,
    pub width: usize,
    pub height: usize,
    pub left: i32,
    pub top: i32,
    pub frames: Vec<usize>,
    pub palette: Palette,
}

struct Image {
    sheet: PlayerSheet,
    scale: usize,
    pixels: Vec<[u8; 4]>,
}
type Cells = Rc<Vec<Rc<Replacement>>>;

#[derive(Default)]
pub(super) struct PlayerArt {
    images: HashMap<usize, Rc<Image>>,
    cache: RefCell<HashMap<usize, Cells>>,
}
impl PlayerArt {
    pub(super) fn load(
        source: &mut Source,
        sheets: Vec<PlayerSheet>,
        total: &mut usize,
    ) -> Result<Self, String> {
        let mut art = Self::default();
        for sheet in sheets {
            if sheet.width == 0
                || sheet.height == 0
                || sheet.width > 128
                || sheet.height > 128
                || sheet.left.abs_diff(0) > 128
                || sheet.top.abs_diff(0) > 128
                || sheet.frames.is_empty()
                || sheet
                    .frames
                    .iter()
                    .any(|&id| id >= PlayerAssets::FRAME_COUNT)
            {
                return Err("invalid player frame bounds or frame ID".into());
            }
            let (width, height, pixels) = image(source, &sheet.file)?;
            *total += pixels.len() * 4;
            if *total > MAX_PACK_BYTES {
                return Err("art pack exceeds 1 GiB of decoded sheets".into());
            }
            let scale = width / sheet.width;
            if !(1..=MAX_SCALE).contains(&scale)
                || sheet.width * scale != width
                || sheet.height * scale != height
            {
                return Err(format!(
                    "{}: player PNG must be 1..10 times its native dimensions",
                    sheet.file
                ));
            }
            let image = Rc::new(Image {
                sheet,
                scale,
                pixels,
            });
            for &id in &image.sheet.frames {
                if art.images.insert(id, image.clone()).is_some() {
                    return Err(format!("duplicate player frame {id}"));
                }
            }
        }
        Ok(art)
    }
    pub(super) fn len(&self) -> usize {
        self.images.len()
    }
    pub(super) fn scale(&self) -> usize {
        self.images
            .values()
            .map(|image| image.scale)
            .max()
            .unwrap_or(1)
    }

    pub(super) fn cells(
        &self,
        id: usize,
        cells: impl Iterator<Item = (i32, i32, Flip)>,
    ) -> Option<Cells> {
        let image = self.images.get(&id)?;
        if let Some(cached) = self.cache.borrow().get(&id) {
            return Some(cached.clone());
        }
        let cells: Vec<_> = cells.collect();
        let sheet = &image.sheet;
        // The first piece in SAT order owns overlapping pixels. A complete
        // redraw is sampled once even where original pieces overlapped.
        let mut owners = vec![usize::MAX; sheet.width * sheet.height];
        for (index, &(x, y, _)) in cells.iter().enumerate() {
            for dy in 0..8 {
                for dx in 0..8 {
                    let (x, y) = (x + dx - sheet.left, y + dy - sheet.top);
                    if x >= 0 && y >= 0 && x < sheet.width as i32 && y < sheet.height as i32 {
                        let owner = &mut owners[y as usize * sheet.width + x as usize];
                        if *owner == usize::MAX {
                            *owner = index;
                        }
                    }
                }
            }
        }
        let side = 8 * image.scale;
        let prepared: Cells = Rc::new(
            cells
                .iter()
                .enumerate()
                .map(|(index, &(tx, ty, flip))| {
                    let mut pixels = vec![[0; 4]; side * side];
                    for y in 0..side {
                        for x in 0..side {
                            let (lx, ly) = (
                                tx + (x / image.scale) as i32 - sheet.left,
                                ty + (y / image.scale) as i32 - sheet.top,
                            );
                            if lx < 0
                                || ly < 0
                                || lx >= sheet.width as i32
                                || ly >= sheet.height as i32
                                || owners[ly as usize * sheet.width + lx as usize] != index
                            {
                                continue;
                            }
                            let sx = (tx - sheet.left) * image.scale as i32 + x as i32;
                            let sy = (ty - sheet.top) * image.scale as i32 + y as i32;
                            let pixel =
                                image.pixels[sy as usize * sheet.width * image.scale + sx as usize];
                            let dx = if flip.x { side - 1 - x } else { x };
                            let dy = if flip.y { side - 1 - y } else { y };
                            pixels[dy * side + dx] = pixel;
                        }
                    }
                    Rc::new(Replacement {
                        scale: image.scale,
                        pixels,
                        palette: sheet.palette,
                    })
                })
                .collect(),
        );
        self.cache.borrow_mut().insert(id, prepared.clone());
        Some(prepared)
    }
}
