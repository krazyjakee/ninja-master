use super::*;
use crate::{
    assets::{PlayerAssets, StageAssets},
    stage::StageId,
};
use std::fmt::Write as _;

/// Export into a fresh directory: existing artwork is never overwritten.
/// Includes every player frame and the selected stage part's loaded patterns.
pub fn export_template(
    rom: &[u8],
    stage: StageId,
    path: &Path,
    scale: usize,
) -> Result<(), String> {
    if !(1..=MAX_SCALE).contains(&scale) {
        return Err("template scale must be 1..10".into());
    }
    let assets = StageAssets::load_stage(rom, stage)?;
    let player = PlayerAssets::load(rom)?;
    let mut manifest = Manifest {
        version: 1,
        name: "My art pack".into(),
        sheets: Vec::new(),
        player_frames: Vec::new(),
    };
    fs::create_dir(path).map_err(|e| format!("choose a new export folder: {e}"))?;
    let result = (|| {
        let mut guide = String::from("Player frame positions (original pixel coordinates):\n");
        let bounds: Vec<_> = player
            .frames
            .iter()
            .map(|frame| {
                let cells: Vec<_> = frame.art_cells().collect();
                let left = cells.iter().map(|c| c.1).min().unwrap_or(0);
                let top = cells.iter().map(|c| c.2).min().unwrap_or(0);
                let right = cells.iter().map(|c| c.1 + 8).max().unwrap_or(8);
                let bottom = cells.iter().map(|c| c.2 + 8).max().unwrap_or(8);
                (left, top, (right - left) as usize, (bottom - top) as usize)
            })
            .collect();
        let cell_width = bounds.iter().map(|b| b.2).max().unwrap_or(8) + 8;
        let cell_height = bounds.iter().map(|b| b.3).max().unwrap_or(8) + 8;
        let row_bytes = cell_width * 8 * cell_height * scale * scale * 4;
        let rows_per_sheet = (MAX_IMAGE_BYTES / row_bytes).max(1);
        let frames_per_sheet = rows_per_sheet * 8;
        for (page, frames) in player.frames.chunks(frames_per_sheet).enumerate() {
            let file = if page == 0 {
                "player.png".to_owned()
            } else {
                format!("player-{}.png", page + 1)
            };
            let mut sheet = Template::new(
                &file,
                cell_width * 8,
                cell_height * frames.len().div_ceil(8),
                assets.palettes,
                scale,
            )?;
            for (offset, frame) in frames.iter().enumerate() {
                let id = page * frames_per_sheet + offset;
                let (left, top, width, height) = bounds[id];
                let (x, y) = (
                    (offset % 8) * cell_width + 4,
                    (offset / 8) * cell_height + 4,
                );
                writeln!(guide, "Frame {id}: file={file}, x={x}, y={y}, width={width}, height={height}, origin=({}, {})", x as i32 - left, y as i32 - top).unwrap();
                for (tile, tx, ty, line, flip) in frame.art_cells() {
                    sheet.cell(
                        tile,
                        line as u8,
                        x + (tx - left) as usize,
                        y + (ty - top) as usize,
                        flip.x,
                        flip.y,
                    );
                }
            }
            manifest.sheets.push(sheet.save(path)?);
        }
        let mut patterns = assets.tiles.clone();
        if let Some(animation) = &assets.effects.animation {
            for (_, tiles) in animation
                .channels
                .iter()
                .flat_map(|channel| &channel.frames)
            {
                for tile in tiles {
                    if !patterns.contains(tile) {
                        patterns.push(*tile);
                    }
                }
            }
        }
        for line in 0..4 {
            let mut sheet = Template::new(
                &format!("stage-line-{line}.png"),
                128,
                patterns.len().div_ceil(16) * 8,
                assets.palettes,
                scale,
            )?;
            for (id, tile) in patterns
                .iter()
                .enumerate()
                .filter(|(_, tile)| tile.0 != [0; 32])
            {
                sheet.cell(tile, line, (id % 16) * 8, (id / 16) * 8, false, false);
            }
            manifest.sheets.push(sheet.save(path)?);
        }
        fs::write(path.join("frames.txt"), guide).map_err(|e| e.to_string())?;
        fs::write(
            path.join("README.txt"),
            format!(
                "Ninja Master art pack template v1\n\n\
             Stage {}-{}, part {}. PNGs exported at {scale}x.\n\n\
             1. Paint over player.png or the stage-line-N.png sheets in any PNG editor.\n\
             2. Keep the sheet dimensions, frame positions and transparent background.\n\
             3. In F1 > Art packs, select this pack from Artwork.\n\n\
             player*.png contain assembled player frames, eight per row. frames.txt\n\
             lists their bounds and feet origins. Stage sheets contain 8x8 source\n\
             tiles, sixteen per row, shown in each of the four palette lines.\n\n\
             For HD art, enlarge a whole sheet to 2x, 4x, 8x or 10x its ORIGINAL size\n\
             before drawing finer detail. JSON positions always use original pixels.\n\
             Sheets may use different integer scales from 1x to 10x. PNG alpha is\n\
             supported. Delete unused sheets from pack.json for a partial pack.\n\n\
             Shared tiles repeat across frames: edit ONE occurrence to change all\n\
             uses. Untouched copies are ignored. Different edits of the same tile\n\
             and palette line are rejected; make those edits identical.\n\n\
             Exact original palette colours follow palette changes; other colours\n\
             are tinted using the nearest original colour. Missing tiles use ROM\n\
             artwork. This template includes loaded and animated stage patterns;\n\
             front-end screens are not included. Export other stages as\n\
             needed and combine their sheets in pack.json.\n\n\
             To share one file, ZIP this folder (or its contents). Drop that ZIP\n\
             into art-packs/ and select it from Artwork; extraction is not needed.\n",
                stage.round, stage.substage, stage.part
            ),
        )
        .map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        fs::write(path.join(MANIFEST), json).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        // This directory was created by this invocation, not supplied content.
        let _ = fs::remove_dir_all(path);
    }
    result
}

struct Template {
    sheet: Sheet,
    scale: usize,
    image: Vec<u8>,
}

impl Template {
    fn new(
        file: &str,
        width: usize,
        height: usize,
        palettes: [Palette; 4],
        scale: usize,
    ) -> Result<Self, String> {
        let bytes = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(scale * scale * 4))
            .filter(|n| *n <= MAX_IMAGE_BYTES)
            .ok_or("template sheet exceeds 64 MiB; choose a smaller export scale")?;
        Ok(Self {
            sheet: Sheet {
                file: file.into(),
                width,
                height,
                palettes,
                cells: Vec::new(),
            },
            scale,
            image: vec![0; bytes],
        })
    }

    fn cell(&mut self, tile: &Tile, line: u8, x: usize, y: usize, flip_x: bool, flip_y: bool) {
        let hex = tile.0.iter().fold(String::new(), |mut s, b| {
            write!(s, "{b:02x}").unwrap();
            s
        });
        self.sheet.cells.push(Cell {
            tile: hex,
            line,
            x,
            y,
            flip_x,
            flip_y,
            baseline: None,
        });
        let width = self.sheet.width * self.scale;
        let side = 8 * self.scale;
        for dy in 0..side {
            for dx in 0..side {
                let sx = (if flip_x { side - 1 - dx } else { dx }) / self.scale;
                let sy = (if flip_y { side - 1 - dy } else { dy }) / self.scale;
                let index = tile_index(&tile.0, sx, sy);
                let color = rgba(
                    self.sheet.palettes[usize::from(line)][usize::from(index)],
                    index != 0,
                );
                let at = ((y * self.scale + dy) * width + x * self.scale + dx) * 4;
                if color[3] != 0 {
                    self.image[at..at + 4].copy_from_slice(&color);
                }
            }
        }
    }

    fn save(mut self, directory: &Path) -> Result<Sheet, String> {
        for cell in &mut self.sheet.cells {
            let bytes = decode_tile(&cell.tile)?;
            let palette = self.sheet.palettes[usize::from(cell.line)];
            let mut baseline = Vec::with_capacity(64);
            let mut composite = false;
            for y in 0..8 {
                for x in 0..8 {
                    let sx = if cell.flip_x { 7 - x } else { x };
                    let sy = if cell.flip_y { 7 - y } else { y };
                    let at = ((cell.y + sy) * self.scale * self.sheet.width * self.scale
                        + (cell.x + sx) * self.scale)
                        * 4;
                    let pixel: [u8; 4] = self.image[at..at + 4].try_into().unwrap();
                    let index = tile_index(&bytes, x, y);
                    let expected = rgba(palette[index as usize], index != 0);
                    composite |=
                        pixel[3] != expected[3] || (pixel[3] != 0 && pixel[..3] != expected[..3]);
                    baseline.push(pixel);
                }
            }
            if composite {
                cell.baseline = Some(baseline);
            }
        }
        write_png(
            &directory.join(&self.sheet.file),
            self.sheet.width * self.scale,
            self.sheet.height * self.scale,
            &self.image,
        )?;
        Ok(self.sheet)
    }
}
