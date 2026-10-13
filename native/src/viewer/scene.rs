//! Stage planes and scenery shared by gameplay and asset inspection.
use super::Viewer;
use crate::{
    HEIGHT,
    combat::draw_pieces,
    raster::SineWave,
    render::{
        CramChange, Flip, Frame, Lines, PaletteRef, SatWriter, cram_lines, cram_palettes, md_color,
    },
    rom::SpritePiece,
    stage::{
        BossPlane, BossRaster, COLUMNS, EVENT_FLAG_TICK, EVENT_PALETTE_TICK, LineWave,
        ScrollInputs, VScroll, Vsram, forest_background_x,
    },
};

/// With parallax disabled every plane B line uses this line's offset (in
/// 1-1, the quarter-speed band).
const UNIFORM_PARALLAX_LINE: usize = 72;

impl Viewer {
    /// Clear padded map columns outside a narrow part's playable scene.
    /// Keep the backdrop's CRAM index so raster palette changes still apply.
    pub(crate) fn clear_world_padding(&self, frame: &mut Frame) {
        let (left, right) = self.world_bounds();
        if frame.width() == crate::WIDTH || right - left >= frame.width() as i32 {
            return;
        }
        let (backdrop, index) = self.backdrop();
        let slot = frame.slot(backdrop);
        let left = (left - self.x).clamp(frame.view_left(), frame.view_right());
        let right = (right - self.x).clamp(frame.view_left(), frame.view_right());
        for y in 0..HEIGHT as i32 {
            for x in (frame.view_left()..left).chain(right..frame.view_right()) {
                frame.index_pixel(x, y, slot, index);
            }
        }
    }

    /// The stage's CRAM lines.
    pub(crate) fn lines(&self) -> Lines<'_> {
        cram_lines(&self.assets.palettes)
    }
    pub(super) fn attr(&self, frame: &mut Frame, attr: u16, x: i32, y: i32) {
        let line = (attr >> 13) & 3;
        frame.tile(
            &self.assets.tiles[usize::from(attr & 0x7ff)],
            PaletteRef::Cram(line as u8, &self.assets.palettes[usize::from(line)]),
            x,
            y,
            Flip {
                x: attr & 0x800 != 0,
                y: attr & 0x1000 != 0,
            },
        );
    }
    pub(crate) fn scene(&self, frame: &mut Frame) {
        self.scene_viewport(frame, 16, 192, None);
        self.front_viewport(frame, 16, 192);
    }
    /// The play viewport under the sprites; the game draws `front_game`
    /// after them.
    /// `scene_game` with the scenery sprites cut at line `sprite_end`
    /// (the 4-1 H-interrupt, `ScrollModel::sprite_cut`).
    pub(crate) fn scene_game_cut(&self, frame: &mut Frame, sprite_end: Option<usize>) {
        self.scene_viewport(frame, 0, HEIGHT, sprite_end);
    }
    /// `Mode_StageLoad` after a lost life: the boss-drawn planes, scroll
    /// and the CRAM lines the boss rewrites (the Hydra's 0..2) as the stage
    /// loads them.
    pub(crate) fn reload_boss_planes(&mut self) {
        self.background_x_bias = 0;
        let data = &self.assets.data;
        self.boss_raster = BossRaster::for_stage(data);
        self.plane_a_wave = None;
        self.final_boss_scroll = None;
        self.final_boss_ticks = 0;
        self.cram_reload = None;
        self.backdrop_entry = 0;
        let lines = self.boss_raster.as_ref().map_or(0, |r| r.cram_lines);
        if lines > 0 {
            self.assets.palette_words[..lines].copy_from_slice(&data.palette_words[..lines]);
            self.assets.palettes = cram_palettes(&self.assets.palette_words);
        }
    }
    /// VDP register `$07`'s backdrop: CRAM line `entry >> 4`, colour
    /// `entry & 15` of the stage's palettes, where no plane or sprite
    /// pixel covers the screen (7-1's sky, the Hydra's laser band that
    /// `Func_02D1FC` flickers through line 0 colour 0).
    pub(crate) fn backdrop(&self) -> (PaletteRef<'_>, u8) {
        let line = usize::from(self.backdrop_entry >> 4) & 3;
        (
            PaletteRef::Cram(line as u8, &self.assets.palettes[line]),
            self.backdrop_entry & 15,
        )
    }
    /// The installed CRAM reload as the frame draws it: the reloaded line
    /// holds the reload's colours from its display line down. V-blank's
    /// upload of the `$FFF000` copy restores the line at the top.
    pub(crate) fn cram_change(&self) -> Option<CramChange> {
        let reload = self.cram_reload?;
        Some(CramChange {
            line: usize::from(reload.line),
            cram_line: reload.cram_line,
            colors: reload.words.map(md_color),
        })
    }
    /// The high-priority cells of both planes, over the sprites.
    pub(crate) fn front_game(&self, frame: &mut Frame) {
        self.front_viewport(frame, 0, HEIGHT);
    }
    /// The VDP layers under the sprites: plane B's low cells (the
    /// recovered name table, 1-1's event map once its flag is set, or a
    /// boss-drawn table), plane A's low cells (the metatile layout, the
    /// horse/surf byte-map overlay or a boss-drawn table), then the scenery
    /// sprites. With the priority effect off both planes are drawn whole
    /// here. Both planes follow the frame's VSRAM columns and HScroll lines
    fn scene_viewport(
        &self,
        frame: &mut Frame,
        top: i32,
        height: usize,
        sprite_end: Option<usize>,
    ) {
        let low = self.effects_enabled[2].then_some(false);
        if self.layer != 1 {
            self.plane_b_layer(frame, top, height, low);
        }
        if self.layer != 2 {
            self.plane_a_layer(frame, top, height, low);
        }
        if self.layer != 1 {
            frame.clip_lines(sprite_end);
            self.scenery(frame);
            frame.clip_lines(None);
        }
    }
    /// Plane B's then plane A's high-priority cells: VDP order puts both
    /// over low-priority sprites, and plane A's over plane B's.
    fn front_viewport(&self, frame: &mut Frame, top: i32, height: usize) {
        if !self.effects_enabled[2] {
            return;
        }
        if self.layer != 1 {
            self.plane_b_layer(frame, top, height, Some(true));
        }
        if self.layer != 2 {
            self.plane_a_layer(frame, top, height, Some(true));
        }
    }
    /// Plane B cells of one priority (`None`: all).
    fn plane_b_layer(&self, frame: &mut Frame, top: i32, height: usize, priority: Option<bool>) {
        if let Some(plane) = self.boss_raster.as_ref().and_then(|r| r.plane_b.as_ref()) {
            self.boss_plane(frame, top, height, plane, priority);
            return;
        }
        let data = &self.assets.data;
        let vsram = self.vsram();
        let v = &vsram.plane_b;
        let delta = frame.world_camera_delta();
        let smooth = frame.smooth_wave_effects;
        let shift = |line| self.plane_b_sample(line, self.x + delta, smooth) - delta as f32;
        let event = self.event_ticks.is_some_and(|t| t >= EVENT_FLAG_TICK);
        match self.assets.effects.event.as_ref().filter(|_| event) {
            // $37116: 16x25 cells repeated horizontally.
            Some(event) => self.name_table_priority(
                frame,
                top,
                height,
                v,
                shift,
                |col, row| {
                    if row < 25 {
                        event.background[row * 16 + col % 16]
                    } else {
                        0
                    }
                },
                priority,
                false,
            ),
            None => self.name_table_priority(
                frame,
                top,
                height,
                v,
                shift,
                |col, row| data.plane_b.table.get(col, row),
                priority,
                false,
            ),
        }
    }
    /// Plane A cells of one priority (`None`: all).
    fn plane_a_layer(&self, frame: &mut Frame, top: i32, height: usize, priority: Option<bool>) {
        let data = &self.assets.data;
        if let Some(plane) = self.boss_raster.as_ref().and_then(|r| r.plane_a.as_ref()) {
            self.boss_plane(frame, top, height, plane, priority);
        } else if let Some(overlay) = &data.plane_a_overlay {
            let vsram = self.vsram();
            let delta = frame.world_camera_delta();
            let smooth = frame.smooth_wave_effects;
            self.name_table_priority(
                frame,
                top,
                height,
                &vsram.plane_a,
                |line| self.plane_a_sample(line, self.x + delta, smooth) - delta as f32,
                |col, row| overlay.get(col, row),
                priority,
                self.final_boss_scroll.is_some_and(|(mode, _)| mode == 7),
            );
        } else {
            self.foreground_viewport(frame, priority, top, height);
        }
    }

    fn foreground_viewport(
        &self,
        frame: &mut Frame,
        priority: Option<bool>,
        top: i32,
        height: usize,
    ) {
        let vsram = self.vsram();
        let lines = top as usize..top as usize + height;
        let first = self.plane_a_offset(lines.start);
        let smooth = frame.smooth_wave_effects;
        if !Vsram::uniform(&vsram.plane_a)
            || lines.clone().any(|l| self.plane_a_offset(l) != first)
            || (smooth
                && (self.assets.data.scroll.routine == 0x207c || self.plane_a_wave.is_some()))
        {
            // Plane A by scanline: its layout row follows each column's
            // word and each line's HScroll.
            self.plane(
                frame,
                top,
                height,
                &vsram.plane_a,
                |line| self.plane_a_sample(line, self.x, smooth),
                |tx, ty| self.layout_cell(tx, ty),
                priority,
                false,
            );
            return;
        }
        {
            let data = &self.assets.data;
            // CameraYShake adds Stage_VScrollOffset (the 7-1 bob).
            let y = if data.scroll.plane_a_v == VScroll::CameraYShake {
                self.y + self.raster.vscroll_offset()
            } else {
                self.y
            };
            let (width, layout_height) = data.pixel_size();
            let last_row = (layout_height / 16 - 1).max(0);
            let last_col = (width / 16 - 1).max(0);
            for row in y / 16..=((y + height as i32 - 1) / 16).min(last_row) {
                let first_col = (self.x + frame.view_left()).div_euclid(16).max(0);
                let end_col = (self.x + frame.view_right() - 1)
                    .div_euclid(16)
                    .min(last_col);
                for col in first_col..=end_col {
                    let Some(id) = data.metatile_at(col, row) else {
                        continue;
                    };
                    let Some(attrs) = data.metatiles.get(usize::from(id)) else {
                        continue;
                    };
                    for (i, attr) in attrs.iter().enumerate() {
                        if priority.is_some_and(|high| (attr & 0x8000 != 0) != high) {
                            continue;
                        }
                        self.attr(
                            frame,
                            *attr,
                            col * 16 - self.x + (i % 2) as i32 * 8,
                            top + row * 16 - y + (i / 2) as i32 * 8,
                        );
                    }
                }
            }
        }
    }

    /// Plane A attribute of layout cell (`tx`, `ty`) in eight-pixel units;
    /// 0 outside the layout.
    fn layout_cell(&self, tx: i32, ty: i32) -> u16 {
        let data = &self.assets.data;
        data.metatile_at(tx.div_euclid(2), ty.div_euclid(2))
            .and_then(|id| data.metatiles.get(usize::from(id)))
            .map_or(0, |attrs| {
                attrs[(ty.rem_euclid(2) * 2 + tx.rem_euclid(2)) as usize]
            })
    }

    /// This frame's VSRAM words.
    pub(crate) fn vsram(&self) -> Vsram {
        if self.final_boss_scroll.is_some_and(|(mode, _)| mode == 7) {
            return crate::stage::ScrollModel::final_boss_vsram(
                self.final_boss_ticks,
                &self.assets.effects.sine,
            );
        }
        let mut vsram = self.assets.data.scroll.vsram(&ScrollInputs {
            camera_y: self.y,
            background_y: self.background_y,
            tick: self.ticks,
            raster: self.raster,
        });
        if let Some(columns) = self.plane_a_vscroll {
            vsram.plane_a = columns;
        }
        vsram
    }

    /// A boss-drawn plane (the 3-3 corridor and Hydra, the 5-3 mech) as
    /// the boss patches and scrolls it: before it writes the scroll the
    /// plane rests at its `BossPlanes` place, following the camera.
    fn boss_plane(
        &self,
        frame: &mut Frame,
        top: i32,
        height: usize,
        plane: &BossPlane,
        priority: Option<bool>,
    ) {
        let mut vscroll = plane.vscroll(self.y);
        let delta = frame.world_camera_delta();
        let smooth = frame.smooth_wave_effects;
        if matches!(
            self.assets.data.boss_planes,
            Some(crate::stage::BossPlanes::Mech { .. })
        ) {
            for column in &mut vscroll {
                *column += self.raster.jolt_offset;
            }
        }
        self.name_table_priority(
            frame,
            top,
            height,
            &vscroll,
            |line| {
                if smooth && let Some((bands, _)) = &plane.written {
                    let row = line as f32 / 8.0;
                    let a = (row.floor() as usize).min(bands.len() - 1);
                    let b = (a + 1).min(bands.len() - 1);
                    -(f32::from(bands[a])
                        + (f32::from(bands[b]) - f32::from(bands[a])) * row.fract())
                        - delta as f32
                } else {
                    (plane.shift(line, self.x + delta) - delta) as f32
                }
            },
            |col, row| plane.table.get(col, row),
            priority,
            plane.written.is_some(),
        );
    }

    /// Draw a 64x64 name table (512x512 pixels, wrapping) by VDP scanline;
    /// `cell` gives the attribute of a column and row in 0..64, and
    /// `priority` selects the cells of one priority (`None`: all).
    #[allow(clippy::too_many_arguments)]
    fn name_table_priority(
        &self,
        frame: &mut Frame,
        top: i32,
        height: usize,
        vscroll: &[i32; COLUMNS],
        shift: impl Fn(usize) -> f32,
        cell: impl Fn(usize, usize) -> u16,
        priority: Option<bool>,
        column_wave: bool,
    ) {
        let wrap = |tx: i32, ty: i32| cell(tx.rem_euclid(64) as usize, ty.rem_euclid(64) as usize);
        self.plane(
            frame,
            top,
            height,
            vscroll,
            shift,
            wrap,
            priority,
            column_wave,
        );
    }

    /// Draw a plane by VDP scanline. `vscroll` holds the plane row shown at
    /// line 0 for each two-cell screen column, `shift` the sample offset
    /// (negated HScroll) of each line and `cell` the attribute of a cell in
    /// eight-pixel plane units. The stages with column scroll keep HScroll
    /// at 0, so screen columns are the VDP's fetch columns.
    #[allow(clippy::too_many_arguments)]
    fn plane(
        &self,
        frame: &mut Frame,
        top: i32,
        height: usize,
        vscroll: &[i32; COLUMNS],
        shift: impl Fn(usize) -> f32,
        cell: impl Fn(i32, i32) -> u16,
        priority: Option<bool>,
        column_wave: bool,
    ) {
        let slots = self.lines().map(|palette| frame.slot(palette));
        let smooth_columns = frame.smooth_wave_effects && column_wave;
        let columns = std::array::from_fn::<_, COLUMNS, _>(|i| {
            if smooth_columns && self.final_boss_scroll.is_some_and(|(mode, _)| mode == 7) {
                SineWave::shifted(self.final_boss_ticks.wrapping_mul(0x40), 0x80, 12)
                    .smooth_offset(i as f32, &self.assets.effects.sine)
                    + 4.0
            } else {
                vscroll[i] as f32
            }
        });
        let hd = frame.art_pack().is_some();
        // Column interpolation depends only on screen X. Reuse it on every
        // scanline instead of repeating the float arithmetic per pixel.
        let native_columns: Vec<f32> = if smooth_columns && !hd {
            (frame.view_left()..frame.view_right())
                .map(|sx| {
                    let position = (sx as f32 / 16.0).clamp(0.0, (COLUMNS - 1) as f32);
                    let column = position.floor() as usize;
                    columns[column]
                        + (columns[(column + 1).min(COLUMNS - 1)] - columns[column])
                            * position.fract()
                })
                .collect()
        } else {
            Vec::new()
        };
        for sy in 0..height {
            let line = sy + top as usize;
            let offset = shift(line);
            if offset.fract() != 0.0 || smooth_columns {
                if hd {
                    let scale = frame.output_scale() as i32;
                    for y in line as i32 * scale..(line as i32 + 1) * scale {
                        let line = y as usize / scale as usize;
                        let offset = shift(line);
                        if !smooth_columns {
                            let mut x = frame.view_left() * scale;
                            let end = frame.view_right() * scale;
                            while x < end {
                                let sx = x as f32 / scale as f32;
                                let column =
                                    (sx / 16.0).floor().clamp(0.0, (COLUMNS - 1) as f32) as usize;
                                let wx = sx + offset;
                                let wy = y as f32 / scale as f32 + columns[column];
                                let run = (((8.0 - wx.rem_euclid(8.0)) * scale as f32).ceil()
                                    as i32)
                                    .max(1)
                                    .min(16 * scale - x.rem_euclid(16 * scale))
                                    .min(end - x);
                                let attr =
                                    cell((wx / 8.0).floor() as i32, (wy / 8.0).floor() as i32);
                                if priority.is_none_or(|high| (attr & 0x8000 != 0) == high) {
                                    frame.hd_tile_span(
                                        &self.assets.tiles[usize::from(attr & 0x7ff)],
                                        slots[usize::from((attr >> 13) & 3)],
                                        (x, y),
                                        (offset, wy.rem_euclid(8.0)),
                                        run as usize,
                                        Flip {
                                            x: attr & 0x800 != 0,
                                            y: attr & 0x1000 != 0,
                                        },
                                    );
                                }
                                x += run;
                            }
                            continue;
                        }
                        for x in frame.view_left() * scale..frame.view_right() * scale {
                            let sx = x as f32 / scale as f32;
                            let position = (sx / 16.0).clamp(0.0, (COLUMNS - 1) as f32);
                            let column = position.floor() as usize;
                            let v = if smooth_columns {
                                columns[column]
                                    + (columns[(column + 1).min(COLUMNS - 1)] - columns[column])
                                        * position.fract()
                            } else {
                                columns[column]
                            };
                            let wx = sx + offset;
                            let wy = y as f32 / scale as f32 + v;
                            let attr = cell((wx / 8.0).floor() as i32, (wy / 8.0).floor() as i32);
                            if priority.is_some_and(|high| (attr & 0x8000 != 0) != high) {
                                continue;
                            }
                            frame.hd_tile_sample(
                                &self.assets.tiles[usize::from(attr & 0x7ff)],
                                slots[usize::from((attr >> 13) & 3)],
                                (x, y),
                                (wx.rem_euclid(8.0), wy.rem_euclid(8.0)),
                                Flip {
                                    x: attr & 0x800 != 0,
                                    y: attr & 0x1000 != 0,
                                },
                            );
                        }
                    }
                    continue;
                }

                for sx in frame.view_left()..frame.view_right() {
                    let v = if smooth_columns {
                        native_columns[(sx - frame.view_left()) as usize]
                    } else {
                        let column = sx.div_euclid(16).clamp(0, COLUMNS as i32 - 1) as usize;
                        columns[column]
                    };
                    let x = sx as f32 + offset;
                    let y = line as f32 + v;
                    // ROM world coordinates stay within i32 range. Truncation
                    // plus a negative-fraction correction gives their floor
                    // without per-pixel libm floor/fmod calls.
                    let tx = x as i32;
                    let ty = y as i32;
                    let tx = tx - i32::from(x < tx as f32);
                    let ty = ty - i32::from(y < ty as f32);
                    let fx = x - tx as f32;
                    let fy = y - ty as f32;
                    let weights = [
                        (1.0 - fx) * (1.0 - fy),
                        fx * (1.0 - fy),
                        (1.0 - fx) * fy,
                        fx * fy,
                    ];
                    let col = tx.div_euclid(8);
                    let row = ty.div_euclid(8);
                    let px = tx.rem_euclid(8) as usize;
                    let py = ty.rem_euclid(8) as usize;
                    // The four taps normally share a cell. Fetch adjacent
                    // attributes only when a tap crosses an eight-pixel edge.
                    let a = cell(col, row);
                    let right = px == 7 && fx != 0.0;
                    let below = py == 7 && fy != 0.0;
                    let b = if right { cell(col + 1, row) } else { a };
                    let c = if below { cell(col, row + 1) } else { a };
                    let d = match (right, below) {
                        (true, true) => cell(col + 1, row + 1),
                        (true, false) => b,
                        (false, true) => c,
                        (false, false) => a,
                    };
                    if priority.is_some_and(|high| {
                        [a, b, c, d]
                            .into_iter()
                            .all(|attr| (attr & 0x8000 != 0) != high)
                    }) {
                        continue;
                    }
                    let samples = [
                        (a, px, py, weights[0]),
                        (b, (px + 1) & 7, py, weights[1]),
                        (c, px, (py + 1) & 7, weights[2]),
                        (d, (px + 1) & 7, (py + 1) & 7, weights[3]),
                    ]
                    .map(|(attr, mut px, mut py, weight)| {
                        // A horizontal-only wave needs two taps, and an
                        // integer sample needs one. Unused taps cannot affect
                        // the blend, including its transparency and priority.
                        if weight == 0.0
                            || priority.is_some_and(|high| (attr & 0x8000 != 0) != high)
                        {
                            return None;
                        }
                        if attr & 0x800 != 0 {
                            px = 7 - px;
                        }
                        if attr & 0x1000 != 0 {
                            py = 7 - py;
                        }
                        let byte = self.assets.tiles[usize::from(attr & 0x7ff)].0[py * 4 + px / 2];
                        let color = if px.is_multiple_of(2) {
                            byte >> 4
                        } else {
                            byte & 15
                        };
                        (color != 0).then_some((slots[usize::from((attr >> 13) & 3)], color))
                    });
                    frame.filtered_pixel(sx, line as i32, samples, weights);
                }
                continue;
            }
            let offset = offset as i32;
            let mut sx = frame.view_left();
            // One run per cell and VSRAM column: both are fixed across it.
            while sx < frame.view_right() {
                // The cartridge supplies 20 columns. Extend the edge words
                // into the extra columns rather than indexing past VSRAM.
                let column = sx.div_euclid(16).clamp(0, COLUMNS as i32 - 1) as usize;
                let world_y = line as i32 + vscroll[column];
                let world_x = sx + offset;
                let first = world_x.rem_euclid(8) as usize;
                let run = (8 - first)
                    .min(16 - sx.rem_euclid(16) as usize)
                    .min((frame.view_right() - sx) as usize);
                let attr = cell(world_x.div_euclid(8), world_y.div_euclid(8));
                if priority.is_none_or(|high| (attr & 0x8000 != 0) == high) {
                    if hd {
                        frame.hd_tile_run(
                            &self.assets.tiles[usize::from(attr & 0x7ff)],
                            slots[usize::from((attr >> 13) & 3)],
                            (sx, line as i32),
                            first,
                            world_y.rem_euclid(8) as usize,
                            run,
                            Flip {
                                x: attr & 0x800 != 0,
                                y: attr & 0x1000 != 0,
                            },
                        );
                        sx += run as i32;
                        continue;
                    }
                    let mut py = world_y.rem_euclid(8) as usize;
                    if attr & 0x1000 != 0 {
                        py = 7 - py;
                    }
                    let row = &self.assets.tiles[usize::from(attr & 0x7ff)].0[py * 4..py * 4 + 4];
                    let slot = slots[usize::from((attr >> 13) & 3)];
                    for i in 0..run {
                        let mut px = first + i;
                        if attr & 0x800 != 0 {
                            px = 7 - px;
                        }
                        let byte = row[px / 2];
                        let color = if px.is_multiple_of(2) {
                            byte >> 4
                        } else {
                            byte & 15
                        };
                        if color != 0 {
                            frame.index_pixel(sx + i as i32, line as i32, slot, color);
                        }
                    }
                }
                sx += run as i32;
            }
        }
    }

    /// `Camera_XFixed` (`$FFE010`), approximated by the camera; 1-1 parts
    /// continue at part*2048 (`$11970`).
    pub(super) fn background_x(&self) -> i32 {
        if self.assets.data.id.is_forest() {
            self.x + self.assets.data.id.part_index() as i32 * 2048
        } else {
            self.x + self.background_x_bias
        }
    }

    /// Horizontal 1-1 plane B sample offset for VDP scanline `line`. Every
    /// plane B consumer (band sampling and scenery sprites) uses this one
    /// rounding rule.
    pub(super) fn background_shift(&self, line: usize) -> i32 {
        self.background_shift_at(line, self.x)
    }

    fn background_shift_at(&self, line: usize, camera_x: i32) -> i32 {
        let line = if !self.effects_enabled[1] {
            UNIFORM_PARALLAX_LINE
        } else if self.event_ticks.is_some_and(|t| t >= EVENT_FLAG_TICK) {
            208 // After the event flag every row uses the 3/4 ratio.
        } else {
            line
        };
        forest_background_x(self.background_x() + camera_x - self.x, line)
    }

    fn final_boss_sample(&self, line: usize, foreground: bool, smooth: bool) -> Option<f32> {
        if !smooth {
            return self
                .final_boss_offset(line, foreground)
                .map(|value| value as f32);
        }
        let (mode, wave) = self.final_boss_scroll?;
        let sine = &self.assets.effects.sine;
        if mode == 3 {
            return Some(
                -SineWave::scaled(wave.phase.wrapping_mul(2), 0x20, wave.amplitude)
                    .smooth_offset(line as f32, sine),
            );
        }
        let Some(i) = line.checked_sub(24).filter(|&i| i < 200) else {
            return Some(0.0);
        };
        let tick = self.final_boss_ticks;
        Some(-if foreground {
            SineWave::shifted(tick.wrapping_mul(0x40), 0x80, 12).smooth_offset(i as f32 / 8.0, sine)
                + 4.0
        } else {
            SineWave::shifted(tick.wrapping_mul(4), 0x13, 9).smooth_offset(i as f32, sine)
                + SineWave::shifted(tick.wrapping_mul(0x23), 0x20, 9).smooth_offset(i as f32, sine)
        })
    }

    fn plane_a_sample(&self, line: usize, camera_x: i32, smooth: bool) -> f32 {
        if let Some(offset) = self.final_boss_sample(line, true, smooth) {
            return offset;
        }
        if !smooth {
            return self.plane_a_offset_at(line, camera_x) as f32;
        }
        let sine = &self.assets.effects.sine;
        let base = if self.assets.data.scroll.routine == 0x207c && line < 224 {
            camera_x as f32
                - SineWave::shifted(self.raster.wave, 0x20, 12)
                    .smooth_offset(line as f32 / 8.0, sine)
        } else {
            self.assets.data.scroll.plane_a_line_offset(
                line,
                camera_x,
                self.ticks,
                &self.raster,
                sine,
            ) as f32
        };
        if let Some(wave) = self.plane_a_wave
            && let Some(i) = line
                .checked_sub(LineWave::FIRST)
                .filter(|&i| i < LineWave::LINES)
        {
            base - SineWave::scaled(wave.phase.wrapping_mul(2), 0x20, wave.amplitude)
                .smooth_offset(i as f32, sine)
        } else {
            base
        }
    }

    fn plane_b_sample(&self, line: usize, camera_x: i32, smooth: bool) -> f32 {
        self.final_boss_sample(line, false, smooth)
            .unwrap_or_else(|| self.plane_b_offset_at(line, camera_x) as f32)
    }

    fn final_boss_offset(&self, line: usize, foreground: bool) -> Option<i32> {
        let (mode, wave) = self.final_boss_scroll?;
        let sine = &self.assets.effects.sine;
        if mode == 3 {
            // $1FB16 writes the same displaced value to both words of every line.
            Some(
                -SineWave::scaled(wave.phase.wrapping_mul(2), 0x20, wave.amplitude)
                    .offset(line, sine),
            )
        } else {
            let (a, b) =
                crate::stage::ScrollModel::final_boss_hscroll(line, self.final_boss_ticks, sine);
            Some(-if foreground { a } else { b })
        }
    }

    /// Plane A sample offset (the negated HScroll) of VDP scanline `line`.
    fn plane_a_offset(&self, line: usize) -> i32 {
        self.plane_a_offset_at(line, self.x)
    }

    fn plane_a_offset_at(&self, line: usize, camera_x: i32) -> i32 {
        if let Some(offset) = self.final_boss_offset(line, true) {
            return offset;
        }
        let effects = &self.assets.effects;
        let offset = self.assets.data.scroll.plane_a_line_offset(
            line,
            camera_x,
            self.ticks,
            &self.raster,
            &effects.sine,
        );
        match self.plane_a_wave {
            Some(wave) => offset - wave.offset(line, &effects.sine),
            None => offset,
        }
    }

    /// Plane B sample offset (the negated HScroll) of VDP scanline `line`.
    fn plane_b_offset(&self, line: usize) -> i32 {
        self.plane_b_offset_at(line, self.x)
    }

    fn plane_b_offset_at(&self, line: usize, camera_x: i32) -> i32 {
        if let Some(offset) = self.final_boss_offset(line, false) {
            return offset;
        }
        if self.assets.data.id.is_forest() {
            return if camera_x == self.x {
                self.background_shift(line)
            } else {
                self.background_shift_at(line, camera_x)
            };
        }
        let line = if self.effects_enabled[1] {
            line
        } else {
            UNIFORM_PARALLAX_LINE
        };
        let scroll = self.assets.data.scroll;
        let vscroll = self.vsram().plane_b[0];
        scroll.plane_b_line_offset(
            line,
            vscroll,
            self.background_x() + camera_x - self.x,
            self.ticks,
        )
    }

    /// `$39A02`: this frame's scenery sprites as (screen X, screen Y, piece),
    /// back to front. The list is appended from the end of the SAT after every
    /// object sprite, so each later piece wins overlaps and all of them sit
    /// behind other sprites. `$39DA0` reads `$FFF082`, plane B's line-0
    /// HScroll: the 1-1 trunks follow the 3/8 canopy band.
    pub(super) fn scenery_sprites(&self) -> impl Iterator<Item = (i32, i32, SpritePiece)> + '_ {
        // $371E4 clears the list with the 1-1 event palette.
        let cleared = self.event_ticks.is_some_and(|t| t >= EVENT_PALETTE_TICK);
        let list = if cleared {
            &[][..]
        } else {
            &self.assets.effects.scenery[..]
        };
        let hscroll = -self.plane_b_offset(0);
        list.iter()
            .filter_map(move |scenery| {
                Some((
                    scenery,
                    scenery.position(
                        hscroll,
                        self.assets.data.id.part_index(),
                        self.raster.vscroll_offset(),
                    )?,
                ))
            })
            .flat_map(|(scenery, (x, y))| {
                // Nine-bit VDP sprite coordinates; 128 is the display edge.
                scenery.pieces.iter().map(move |piece| {
                    (
                        ((x + piece.x) & 0x1ff) - 128,
                        ((y + piece.y) & 0x1ff) - 128,
                        *piece,
                    )
                })
            })
    }

    pub(super) fn scenery(&self, frame: &mut Frame) {
        let tiles = &self.assets.tiles;
        let delta = frame.world_camera_delta();
        let shift = self.plane_b_offset(0) - self.plane_b_offset_at(0, self.x + delta) + delta;
        for (x, y, piece) in self.scenery_sprites() {
            let piece = SpritePiece {
                x: 0,
                y: 0,
                ..piece
            };
            draw_pieces(
                frame,
                std::slice::from_ref(&piece),
                &self.lines(),
                0,
                (x + shift, y),
                false,
                SatWriter::Scenery,
                |tile| Some(&tiles[tile & 0x7ff]),
            );
        }
    }
}
