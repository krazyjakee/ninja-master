use super::*;

impl Frame {
    /// Keep simulation coordinates native while resolving actual HD texels.
    pub fn set_art_pack(&mut self, pack: Option<Rc<ArtPack>>) {
        self.reset_art_cache();
        self.art_pack = pack;
        self.hd = self.art_pack.as_ref().map(|pack| {
            let size = self.width * HEIGHT * pack.scale * pack.scale;
            Hd {
                scale: pack.scale,
                pixels: vec![direct([0; 3]); size],
                high: vec![false; size],
                rgb: OnceCell::new(),
            }
        });
        self.clear([0; 3]);
    }

    /// Sample HD sources at the displayed integer scale. Captures default to
    /// the pack's full resolution unless the desktop sets a smaller viewport.
    pub fn set_output_scale(&mut self, scale: usize) -> bool {
        let Some(hd) = &mut self.hd else {
            return false;
        };
        let scale = scale.clamp(1, self.art_pack.as_ref().unwrap().scale);
        if hd.scale == scale {
            return false;
        }
        hd.scale = scale;
        hd.pixels
            .resize(self.width * HEIGHT * scale * scale, direct([0; 3]));
        hd.high.resize(hd.pixels.len(), false);
        self.clear([0; 3]);
        true
    }

    pub(super) fn reset_art_cache(&mut self) {
        self.tints.clear();
        self.tint_entries.clear();
        self.art_entries.clear();
        self.art_cache_bytes = 0;
        self.art_cache_dirty = false;
    }

    pub(super) fn replacement(&self, tile: &Tile, slot: Slot) -> Option<Rc<Replacement>> {
        self.art_pack
            .as_ref()?
            .replacement(tile, self.slots[slot.0 as usize].cram)
    }

    fn replacement_entries(&mut self, slot: Slot, art: &Replacement, scale: usize) -> ArtEntries {
        // ArtPack owns each immutable replacement for its whole lifetime;
        // switching packs clears this cache before any address can be reused.
        let key = (art as *const Replacement as usize, slot.0, scale);
        if let Some(entries) = self.art_entries.get(&key) {
            return entries.clone();
        }
        let mut colors = HashMap::new();
        let side = 8 * scale;
        let pixels: Vec<_> = (0..side * side)
            .map(|at| {
                let x = at % side * art.scale / scale;
                let y = at / side * art.scale / scale;
                let pixel = art.pixels[y * 8 * art.scale + x];
                let entry = *colors.entry(pixel).or_insert_with(|| {
                    if pixel[3] == 0 {
                        return 0;
                    }
                    let color = [pixel[0], pixel[1], pixel[2]];
                    let nearest = (1..16)
                        .filter(|&i| art.palette[i] != [0; 3])
                        .min_by_key(|&i| {
                            color
                                .iter()
                                .zip(art.palette[i])
                                .map(|(&a, b)| (i32::from(a) - i32::from(b)).pow(2))
                                .sum::<i32>()
                        })
                        .unwrap_or(1);
                    let entry = slot.0 << 4 | nearest as u32;
                    if color == art.palette[nearest] {
                        return entry;
                    }
                    let tint = PixelTint {
                        color,
                        reference: art.palette[nearest],
                        entry,
                    };
                    *self.tint_entries.entry(tint).or_insert_with(|| {
                        let entry = TINT | self.tints.len() as u32;
                        self.tints.push(tint);
                        entry
                    })
                });
                (entry, pixel[3])
            })
            .collect();
        let row_alpha = pixels
            .chunks_exact(side)
            .map(|row| {
                if row.iter().all(|p| p.1 == 255) {
                    255
                } else if row.iter().all(|p| p.1 == 0) {
                    0
                } else {
                    128
                }
            })
            .collect();
        let values = pixels.iter().map(|p| p.0).collect();
        let entries = Rc::new(EncodedArt {
            scale,
            pixels,
            values,
            row_alpha,
        });
        self.art_entries.insert(key, entries.clone());
        self.art_cache_bytes +=
            entries.len() * (std::mem::size_of::<(u32, u8)>() + 4) + entries.row_alpha.len();
        entries
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn art_tile_rows(
        &mut self,
        art: &Replacement,
        slot: Slot,
        x: i32,
        y: i32,
        flip: Flip,
        rows: u8,
    ) {
        let scale = self.output_scale();
        let entries = self.replacement_entries(slot, art, scale);
        let side = 8 * scale;
        self.invalidate();
        let x = x.saturating_add(self.origin_x);
        for dy in 0..side {
            let ly = dy / scale;
            let py = y.saturating_mul(scale as i32).saturating_add(dy as i32);
            if rows & (1 << ly) == 0 || !(0..self.output_height() as i32).contains(&py) {
                continue;
            }
            for dx in 0..side {
                let px = x.saturating_mul(scale as i32).saturating_add(dx as i32);
                if !(0..self.output_width() as i32).contains(&px) {
                    continue;
                }
                let sx = if flip.x { side - 1 - dx } else { dx };
                let sy = if flip.y { side - 1 - dy } else { dy };
                let (entry, alpha) = entries[sy * 8 * entries.scale + sx];
                self.put_hd(px as usize, py as usize, entry, alpha);
            }
        }
    }

    fn put_hd(&mut self, x: usize, y: usize, entry: u32, alpha: u8) {
        let Some(hd) = &self.hd else {
            return;
        };
        if y / hd.scale * self.width + x / hd.scale >= self.end {
            return;
        }
        let at = y * self.width * hd.scale + x;
        self.put_hd_at(at, entry, alpha);
    }

    /// A texel in a run already clipped to the native sprite line limit.
    #[inline(always)]
    fn put_hd_at(&mut self, at: usize, entry: u32, alpha: u8) {
        if alpha == 0 {
            return;
        }
        let hd = self.hd.as_mut().unwrap();
        if self.behind && hd.high[at] {
            return;
        }
        let entry = if alpha == 255 {
            entry
        } else {
            let blend = BLEND | self.blends.len() as u32;
            let weight = f32::from(alpha) / 255.0;
            self.blends.push(PixelBlend {
                entries: [entry, hd.pixels[at], 0, 0],
                weights: [weight, 1.0 - weight, 0.0, 0.0],
            });
            blend
        };
        hd.pixels[at] = entry;
        hd.high[at] = self.marking;
    }

    fn copy_hd_opaque(&mut self, output: usize, entries: &[u32]) {
        let hd = self.hd.as_mut().unwrap();
        let end = output + entries.len();
        if !self.behind || !hd.high[output..end].iter().any(|&v| v) {
            hd.pixels[output..end].copy_from_slice(entries);
            hd.high[output..end].fill(self.marking);
        } else {
            for (dx, &entry) in entries.iter().enumerate() {
                if !hd.high[output + dx] {
                    hd.pixels[output + dx] = entry;
                    hd.high[output + dx] = self.marking;
                }
            }
        }
    }

    /// One native scanline run; unchanged art expands with the ordinary
    /// indexed write, while replacement art samples each HD texel.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hd_tile_run(
        &mut self,
        tile: &Tile,
        slot: Slot,
        screen: (i32, i32),
        first: usize,
        row: usize,
        run: usize,
        flip: Flip,
    ) {
        let art = self.replacement(tile, slot);
        let entries = art
            .as_ref()
            .map(|art| self.replacement_entries(slot, art, art.scale));
        let scale = self.output_scale();
        self.invalidate();
        let x = screen.0.saturating_add(self.origin_x);
        let y = screen.1;
        if !(0..HEIGHT as i32).contains(&y) {
            return;
        }
        let left = x.saturating_neg().clamp(0, run as i32) as usize;
        let right = run.min((self.width as i32 - x).max(0) as usize).min(
            self.end
                .saturating_sub(y as usize * self.width)
                .saturating_sub(x.max(0) as usize)
                + left,
        );
        if left >= right {
            return;
        }
        for dx in left..right {
            let px = x + dx as i32;
            if !(0..self.width as i32).contains(&px) {
                continue;
            }
            let tx = if flip.x { 7 - first - dx } else { first + dx };
            let ty = if flip.y { 7 - row } else { row };
            let packed = tile.0[ty * 4 + tx / 2];
            let index = if tx.is_multiple_of(2) {
                packed >> 4
            } else {
                packed & 15
            };
            let at = y as usize * self.width + px as usize;
            if art.is_some() {
                if index != 0 {
                    self.put_native(at, slot.0 << 4 | u32::from(index));
                }
            } else if index != 0 {
                self.put(at, slot.0 << 4 | u32::from(index));
            }
        }
        if let (Some(art), Some(entries)) = (&art, &entries) {
            let output_width = self.width * scale;
            let source_side = art.scale * 8;
            let first_x = (first + left) * scale;
            let count = (right - left) * scale;
            for sy in 0..scale {
                let ty = row * scale + sy;
                let ty = if flip.y { 8 * scale - 1 - ty } else { ty };
                let source_row = ty * art.scale / scale * source_side;
                let output =
                    (y as usize * scale + sy) * output_width + (x + left as i32) as usize * scale;
                if entries.row_alpha[source_row / source_side] == 0 {
                    continue;
                }
                if art.scale == scale {
                    if !flip.x && entries.row_alpha[source_row / source_side] == 255 {
                        self.copy_hd_opaque(
                            output,
                            &entries.values[source_row + first_x..source_row + first_x + count],
                        );
                        continue;
                    }
                    for dx in 0..count {
                        let tx = first_x + dx;
                        let tx = if flip.x { source_side - 1 - tx } else { tx };
                        let (entry, alpha) = entries[source_row + tx];
                        self.put_hd_at(output + dx, entry, alpha);
                    }
                } else {
                    for dx in 0..count {
                        let tx = first_x + dx;
                        let tx = if flip.x { 8 * scale - 1 - tx } else { tx };
                        let (entry, alpha) = entries[source_row + tx * art.scale / scale];
                        self.put_hd_at(output + dx, entry, alpha);
                    }
                }
            }
        }
    }

    /// A physical scanline span whose samples share a tile. Fractional UVs
    /// retain HD detail; column waves may change the source row within a span.
    pub(crate) fn hd_tile_span(
        &mut self,
        tile: &Tile,
        slot: Slot,
        screen: (i32, i32),
        samples: impl ExactSizeIterator<Item = (f32, f32)>,
        flip: Flip,
    ) {
        let scale = self.output_scale();
        let width = self.output_width();
        let (x, y) = (screen.0 + self.origin_x * scale as i32, screen.1);
        if !(0..self.output_height() as i32).contains(&y) {
            return;
        }
        let run = samples.len();
        let left = x.saturating_neg().clamp(0, run as i32) as usize;
        let native_row = y as usize / scale * self.width;
        let end_x = self.end.saturating_sub(native_row).min(self.width) * scale;
        let right = run.min(end_x.saturating_sub(x.max(0) as usize) + left);
        if left >= right || x >= width as i32 {
            return;
        }
        self.invalidate();
        let art = self.replacement(tile, slot);
        let entries = art
            .as_ref()
            .map(|art| self.replacement_entries(slot, art, art.scale));
        let base = slot.0 << 4;
        let output = y as usize * width + (x + left as i32) as usize;
        let source_scale = art.as_ref().map_or(1, |art| art.scale);
        let side = source_scale * 8;
        let native_line = (y as usize).is_multiple_of(scale);
        for (dx, (u, v)) in samples.skip(left).take(right - left).enumerate() {
            let physical_x = screen.0 + (left + dx) as i32;
            let native_sample = native_line && physical_x.rem_euclid(scale as i32) == 0;
            let index = if entries.is_none() || native_sample {
                // UVs are nonnegative, so the integer conversion also floors.
                let tx = (u as usize).min(7);
                let ty = (v as usize).min(7);
                let tx = if flip.x { 7 - tx } else { tx };
                let ty = if flip.y { 7 - ty } else { ty };
                let byte = tile.0[ty * 4 + tx / 2];
                if tx.is_multiple_of(2) {
                    byte >> 4
                } else {
                    byte & 15
                }
            } else {
                0
            };
            let (entry, alpha) = if let Some(entries) = &entries {
                let sx = ((u * source_scale as f32) as usize).min(side - 1);
                let sy = ((v * source_scale as f32) as usize).min(side - 1);
                let sx = if flip.x { side - 1 - sx } else { sx };
                let sy = if flip.y { side - 1 - sy } else { sy };
                entries[sy * side + sx]
            } else {
                (base | u32::from(index), if index == 0 { 0 } else { 255 })
            };
            self.put_hd_at(output + dx, entry, alpha);
            if native_sample && index != 0 {
                self.put_native(
                    native_row + (x + (left + dx) as i32) as usize / scale,
                    base | u32::from(index),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scalar oracle: the old HD sampler, with a fresh artwork lookup so the
    /// test does not share the span renderer's preparation or sampling code.
    fn scalar_hd_sample(
        frame: &mut Frame,
        tile: &Tile,
        slot: Slot,
        screen: (i32, i32),
        uv: (f32, f32),
        flip: Flip,
    ) {
        let scale = frame.output_scale();
        let x = screen.0 + frame.origin_x * scale as i32;
        let y = screen.1;
        if !(0..frame.output_width() as i32).contains(&x)
            || !(0..frame.output_height() as i32).contains(&y)
        {
            return;
        }
        frame.invalidate();
        let art = frame.replacement(tile, slot);
        let entries = art
            .as_ref()
            .map(|art| frame.replacement_entries(slot, art, art.scale));
        if let (Some(art), Some(entries)) = (art, entries) {
            let side = 8 * art.scale;
            let px = (uv.0 * art.scale as f32)
                .floor()
                .clamp(0.0, (side - 1) as f32) as usize;
            let py = (uv.1 * art.scale as f32)
                .floor()
                .clamp(0.0, (side - 1) as f32) as usize;
            let px = if flip.x { side - 1 - px } else { px };
            let py = if flip.y { side - 1 - py } else { py };
            let (entry, alpha) = entries[py * side + px];
            frame.put_hd(x as usize, y as usize, entry, alpha);
        } else {
            let px = uv.0.floor().clamp(0.0, 7.0) as usize;
            let py = uv.1.floor().clamp(0.0, 7.0) as usize;
            let px = if flip.x { 7 - px } else { px };
            let py = if flip.y { 7 - py } else { py };
            let byte = tile.0[py * 4 + px / 2];
            let index = if px % 2 == 0 { byte >> 4 } else { byte & 15 };
            if index != 0 {
                frame.put_hd(x as usize, y as usize, slot.0 << 4 | u32::from(index), 255);
            }
        }
        if screen.0.rem_euclid(scale as i32) == 0 && y.rem_euclid(scale as i32) == 0 {
            let px = uv.0.floor().clamp(0.0, 7.0) as usize;
            let py = uv.1.floor().clamp(0.0, 7.0) as usize;
            let px = if flip.x { 7 - px } else { px };
            let py = if flip.y { 7 - py } else { py };
            let byte = tile.0[py * 4 + px / 2];
            let index = if px % 2 == 0 { byte >> 4 } else { byte & 15 };
            if index != 0 {
                frame.put_native(
                    y as usize / scale * frame.width + x as usize / scale,
                    slot.0 << 4 | u32::from(index),
                );
            }
        }
    }

    #[test]
    fn hd_spans_match_scalar_sampling_with_replacements_and_native_fallbacks() {
        let tile = Tile(std::array::from_fn(|i| (i * 37) as u8));
        let palette: Palette = std::array::from_fn(|i| [(i * 17) as u8, (255 - i * 13) as u8, 144]);
        let path = std::env::temp_dir().join(format!(
            "ninja-wave-span-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        let file = std::fs::File::create(path.join("tile.png")).unwrap();
        let mut png = png::Encoder::new(file, 24, 24);
        png.set_color(png::ColorType::Rgba);
        let mut png = png.write_header().unwrap();
        let pixels: Vec<u8> = (0..24 * 24)
            .flat_map(|i| [((i * 13) % 256) as u8, 72, 216, [0, 128, 255][i % 3]])
            .collect();
        png.write_image_data(&pixels).unwrap();
        drop(png);
        std::fs::write(path.join("pack.json"), serde_json::to_vec(&serde_json::json!({
            "version": 1, "name": "wave test", "sheets": [{
                "file": "tile.png", "width": 8, "height": 8,
                "palettes": ([palette; 4]),
                "cells": [{"tile": tile.0.iter().map(|b| format!("{b:02x}")).collect::<String>(), "line": 0, "x": 0, "y": 0}]
            }]
        })).unwrap()).unwrap();
        let pack = Rc::new(ArtPack::load(&path).unwrap());
        std::fs::remove_dir_all(path).unwrap();
        assert_eq!(pack.replacement_count(), 1);
        let fallback = Tile(std::array::from_fn(|i| (i * 19 + 3) as u8));
        for scale in [1, 2, 3] {
            for tile in [&tile, &fallback] {
                for flip in [
                    Flip::default(),
                    Flip { x: true, y: false },
                    Flip { x: false, y: true },
                    Flip { x: true, y: true },
                ] {
                    let prepare = || {
                        let mut frame = Frame::new(crate::WIDE_WIDTH);
                        frame.set_art_pack(Some(pack.clone()));
                        frame.set_output_scale(scale);
                        frame.clear([12, 34, 56]);
                        let slot = frame.slot(PaletteRef::Cram(0, &palette));
                        frame.mark_high_sprites(true);
                        for x in (frame.view_left()..frame.view_right()).step_by(3) {
                            frame.pixel(x, 31, [255; 3]);
                        }
                        frame.mark_high_sprites(false);
                        frame.behind_high_sprites(true);
                        frame.clip_lines(Some(33));
                        (frame, slot)
                    };
                    let (mut actual, slot) = prepare();
                    let (mut expected, expected_slot) = prepare();
                    let samples: Vec<_> = (0..8 * scale)
                        .map(|i| {
                            (
                                i as f32 / scale as f32 + 0.125,
                                ((i * 5) % (8 * scale)) as f32 / scale as f32,
                            )
                        })
                        .collect();
                    for y in [31 * scale, 32 * scale + scale - 1, 33 * scale] {
                        for x in [
                            actual.view_left() * scale as i32 - 2,
                            actual.view_right() * scale as i32 - 4,
                        ] {
                            actual.hd_tile_span(
                                tile,
                                slot,
                                (x, y as i32),
                                samples.iter().copied(),
                                flip,
                            );
                            for (dx, &uv) in samples.iter().enumerate() {
                                scalar_hd_sample(
                                    &mut expected,
                                    tile,
                                    expected_slot,
                                    (x + dx as i32, y as i32),
                                    uv,
                                    flip,
                                );
                            }
                        }
                    }
                    for frame in [&mut actual, &mut expected] {
                        frame.change_cram_line(CramChange {
                            line: 32,
                            cram_line: 0,
                            colors: [[216, 144, 72]; 16],
                        });
                        frame.fade(Fade::Out(3));
                    }
                    assert!(
                        actual.rgb() == expected.rgb(),
                        "native samples, scale {scale}"
                    );
                    assert!(
                        actual.presentation_rgb() == expected.presentation_rgb(),
                        "HD samples, scale {scale}"
                    );
                    assert_eq!(actual.high, expected.high);
                    assert_eq!(
                        actual.hd.as_ref().unwrap().high,
                        expected.hd.as_ref().unwrap().high
                    );
                }
            }
        }
    }
}
