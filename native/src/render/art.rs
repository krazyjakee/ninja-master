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
        self.sampled_art = None;
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

    /// A scrolled plane sample at a physical output pixel. UV stays in
    /// original 8x8 tile units, so fractional wave motion keeps HD detail.
    pub(crate) fn hd_tile_sample(
        &mut self,
        tile: &Tile,
        slot: Slot,
        screen: (i32, i32),
        uv: (f32, f32),
        flip: Flip,
    ) {
        let scale = self.output_scale();
        let x = screen.0.saturating_add(self.origin_x * scale as i32);
        let y = screen.1;
        if !(0..self.output_width() as i32).contains(&x)
            || !(0..self.output_height() as i32).contains(&y)
        {
            return;
        }
        self.invalidate();
        let key = Key(tile.0, self.slots[slot.0 as usize].cram.unwrap());
        if self
            .sampled_art
            .as_ref()
            .is_none_or(|cached| cached.key != key || cached.slot != slot.0)
        {
            let art = self.replacement(tile, slot);
            let scale = art.as_ref().map_or(1, |a| a.scale);
            let entries = art.map(|a| self.replacement_entries(slot, &a, a.scale));
            self.sampled_art = Some(SampledArt {
                key,
                slot: slot.0,
                entries,
                scale,
            });
        }
        let art = self.sampled_art.as_ref().unwrap();
        if let Some(entries) = &art.entries {
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
            self.put_hd(x as usize, y as usize, entry, alpha);
        } else {
            let px = uv.0.floor().clamp(0.0, 7.0) as usize;
            let py = uv.1.floor().clamp(0.0, 7.0) as usize;
            let px = if flip.x { 7 - px } else { px };
            let py = if flip.y { 7 - py } else { py };
            let byte = tile.0[py * 4 + px / 2];
            let index = if px.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 15
            };
            if index != 0 {
                self.put_hd(x as usize, y as usize, slot.0 << 4 | u32::from(index), 255);
            }
        }
        // Native-resolution tooling remains usable with packs installed.
        if screen.0.rem_euclid(scale as i32) == 0 && screen.1.rem_euclid(scale as i32) == 0 {
            let px = uv.0.floor().clamp(0.0, 7.0) as usize;
            let py = uv.1.floor().clamp(0.0, 7.0) as usize;
            let px = if flip.x { 7 - px } else { px };
            let py = if flip.y { 7 - py } else { py };
            let byte = tile.0[py * 4 + px / 2];
            let index = if px.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 15
            };
            if index != 0 {
                self.put_native(
                    y as usize / scale * self.width + x as usize / scale,
                    slot.0 << 4 | u32::from(index),
                );
            }
        }
    }

    /// One HD scanline span with a constant tile and vertical scroll. The
    /// plane splits spans at tile and VSRAM column boundaries before calling.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hd_tile_span(
        &mut self,
        tile: &Tile,
        slot: Slot,
        screen: (i32, i32),
        source: (f32, f32),
        run: usize,
        flip: Flip,
    ) {
        let scale = self.output_scale();
        let width = self.output_width();
        let (x, y) = (screen.0 + self.origin_x * scale as i32, screen.1);
        if !(0..self.output_height() as i32).contains(&y) {
            return;
        }
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
        let native_y = source.1.floor().clamp(0.0, 7.0) as usize;
        let native_y = if flip.y { 7 - native_y } else { native_y };
        let output = y as usize * width + (x + left as i32) as usize;
        let side = art.as_ref().map_or(8, |art| art.scale * 8);
        let source_scale = art.as_ref().map_or(1, |art| art.scale);
        let source_y = (source.1 * source_scale as f32)
            .floor()
            .clamp(0.0, (side - 1) as f32) as usize;
        let source_y = if flip.y {
            side - 1 - source_y
        } else {
            source_y
        };
        for dx in left..right {
            let physical_x = screen.0 + dx as i32;
            let u = (physical_x as f32 / scale as f32 + source.0).rem_euclid(8.0);
            let tx = u.floor().clamp(0.0, 7.0) as usize;
            let tx = if flip.x { 7 - tx } else { tx };
            let byte = tile.0[native_y * 4 + tx / 2];
            let index = if tx.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 15
            };
            let (entry, alpha) = if let Some(entries) = &entries {
                let sx = (u * source_scale as f32)
                    .floor()
                    .clamp(0.0, (side - 1) as f32) as usize;
                let sx = if flip.x { side - 1 - sx } else { sx };
                entries[source_y * side + sx]
            } else {
                (base | u32::from(index), if index == 0 { 0 } else { 255 })
            };
            self.put_hd_at(output + dx - left, entry, alpha);
            if physical_x.rem_euclid(scale as i32) == 0
                && y.rem_euclid(scale as i32) == 0
                && index != 0
            {
                self.put_native(
                    native_row + (x + dx as i32) as usize / scale,
                    base | u32::from(index),
                );
            }
        }
    }
}
