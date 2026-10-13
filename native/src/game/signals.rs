//! Stage mutations requested by combat and object handlers.
use super::{Game, REQUEST_LIFE_LOST, REQUEST_NEXT_SUBSTAGE, markers};
use crate::movement::NATIVE_UNIT as UNIT;
use crate::render::{cram_palettes, fade_step};

impl Game {
    /// What the object handlers asked for this update: requests, flags and
    /// CRAM and plane changes now; items and layout writes with the stage
    /// objects' update (`tick_objects`).
    pub(super) fn apply_signals(&mut self, signals: &[crate::combat::Signal]) {
        use crate::combat::Signal;
        for &signal in signals {
            match signal {
                Signal::NextSubstage => self.requests |= REQUEST_NEXT_SUBSTAGE,
                Signal::GameFlags(bits) => self.game_flags |= bits,
                Signal::ClampPlayer => {
                    self.clamp_player_to_screen();
                    self.combat.view_screen_clamped = true;
                    self.stage.camera_x_limits = Some(self.view_camera_limits());
                }
                Signal::PlayerMaxX(x) => {
                    self.player.x = self.player.x.min(x * crate::movement::NATIVE_UNIT)
                }
                Signal::CameraMaxX(x) => {
                    let part = self.part();
                    self.bounds[part].x.1 = x;
                    self.stage.camera_x_limits = Some(self.view_camera_limits());
                }
                Signal::VScrollOffset(offset) => self.stage.raster.jolt_offset = offset,
                Signal::Palette(_) | Signal::Plane(_) => {}
                Signal::PlaneAVScroll(columns) => self.stage.plane_a_vscroll = Some(columns),
                Signal::Drop(_) | Signal::Terrain(_) | Signal::MechHands(_) => {
                    self.object_signals.push(signal)
                }
                Signal::Marker(d6, d5) => self.markers.add(markers::Marker::from_words(d6, d5)),
                Signal::PushPlayer(pixels) => {
                    self.player.x += pixels * crate::movement::NATIVE_UNIT;
                }
                Signal::PushVelocity(push) => self.player.push = push,
                Signal::HoldPlayer((x, y)) => {
                    self.player.x = x * UNIT + self.player.x.rem_euclid(UNIT);
                    self.player.y = y * UNIT + self.player.y.rem_euclid(UNIT);
                    self.player.velocity_x = 0;
                    self.player.velocity_y = 0;
                    self.player.accel_y = 0;
                }
                Signal::SwapLeftRight => self.pad.swap = true,
                Signal::TurnPlayer => self.player.facing_left = !self.player.facing_left,
                Signal::BlockNinjutsu => self.pad.ninjutsu_blocked = true,
                // `crate::special` owns the autoscroll; `Combat` applies
                // the priority raise itself.
                Signal::StopAutoscroll | Signal::RaisePlayerPriority | Signal::Shove(_) => {}
            }
        }
        self.apply_palette_ops(signals.iter().filter_map(|signal| match signal {
            Signal::Palette(op) => Some(op),
            _ => None,
        }));
        self.apply_plane_ops(signals.iter().filter_map(|signal| match signal {
            Signal::Plane(op) => Some(op),
            _ => None,
        }));
    }
    pub(super) fn apply_palette_ops<'a>(
        &mut self,
        ops: impl IntoIterator<Item = &'a crate::combat::PaletteOp>,
    ) {
        let mut changed = false;
        for &op in ops {
            changed = true;
            use crate::combat::PaletteOp;
            match op {
                PaletteOp::FadeFirst(line, count, target) => {
                    for (color, target) in self.stage.assets.palette_words[usize::from(line)]
                        .iter_mut()
                        .zip(target)
                        .take(usize::from(count))
                    {
                        *color = fade_step(*color, target);
                    }
                }
                PaletteOp::FadeColor(line, index, target) => {
                    let color =
                        &mut self.stage.assets.palette_words[usize::from(line)][usize::from(index)];
                    *color = fade_step(*color, target);
                }
                PaletteOp::FadeWhite(line)
                | PaletteOp::FadeTo(line, _)
                | PaletteOp::Restore(line) => {
                    if let Some(colors) = self.stage.assets.palette_words.get_mut(line as usize) {
                        let target = match op {
                            PaletteOp::FadeTo(_, target) => target,
                            PaletteOp::Restore(_) => {
                                self.stage.assets.data.palette_words[line as usize]
                            }
                            // $75C fades the software CRAM nibbles to $FFF;
                            // the VDP masks the low bits for display. Keeping
                            // them makes the first restoring step $EEE.
                            _ => [0x0fff; 16],
                        };
                        for (color, target) in colors.iter_mut().zip(target) {
                            *color = fade_step(*color, target);
                        }
                    }
                }
                PaletteOp::Fill(line, color) => {
                    if let Some(colors) = self.stage.assets.palette_words.get_mut(line as usize) {
                        colors.fill(color);
                    }
                }
                PaletteOp::FillFirst(line, count, color) => {
                    if let Some(colors) = self.stage.assets.palette_words.get_mut(line as usize) {
                        let end = usize::from(count).min(colors.len());
                        colors[..end].fill(color);
                    }
                }
                PaletteOp::Load(line, colors) => {
                    if let Some(line) = self.stage.assets.palette_words.get_mut(line as usize) {
                        *line = colors;
                    }
                }
                PaletteOp::Words(line, first, words) => {
                    if let Some(colors) = self
                        .stage
                        .assets
                        .palette_words
                        .get_mut(line as usize)
                        .and_then(|line| line.get_mut(usize::from(first)..usize::from(first) + 8))
                    {
                        colors.copy_from_slice(&words);
                    }
                }
                PaletteOp::Color(line, index, color) => {
                    if let Some(slot) = self
                        .stage
                        .assets
                        .palette_words
                        .get_mut(line as usize)
                        .and_then(|line| line.get_mut(usize::from(index)))
                    {
                        *slot = color;
                    }
                }
                PaletteOp::Rotate(line, first, count) => {
                    if let Some(colors) = self
                        .stage
                        .assets
                        .palette_words
                        .get_mut(line as usize)
                        .and_then(|line| {
                            line.get_mut(usize::from(first)..usize::from(first + count))
                        })
                    {
                        colors.rotate_right(1);
                    }
                }
                PaletteOp::HInt(reload) => {
                    // $2D9E8 keeps IRQ_Default while the player's state is
                    // $40 or more, a fade runs (`Sys_Flags` bit 3), a life
                    // is lost (`Game_Requests` bit 0) or `$FFF020` (CRAM
                    // line 1 colour 0) is black.
                    let quiet = self.player.animator.state >= 0x40
                        || self.fade.is_some()
                        || self.requests & REQUEST_LIFE_LOST != 0
                        || self.stage.assets.palette_words[1][0] == 0;
                    self.stage.cram_reload = reload.filter(|_| !quiet);
                }
                PaletteOp::Backdrop(entry) => self.stage.backdrop_entry = entry & 0x3f,
                PaletteOp::Reload(line) => {
                    let index = usize::from(line);
                    if index < 4 {
                        self.stage.assets.palette_words[index] =
                            self.stage.assets.data.palette_words[index];
                    }
                }
            }
        }
        if changed {
            self.stage.assets.palettes = cram_palettes(&self.stage.assets.palette_words);
        }
    }
    /// Boss plane writes and raster scroll, onto the live boss-drawn planes
    /// (Boss2's wave onto the stage's plane A).
    pub(super) fn apply_plane_ops<'a>(
        &mut self,
        ops: impl IntoIterator<Item = &'a crate::combat::PlaneOp>,
    ) {
        use crate::combat::PlaneOp;
        for &op in ops {
            match op {
                PlaneOp::Wave { amplitude, phase } => {
                    self.stage.plane_a_wave = Some(crate::stage::LineWave { amplitude, phase });
                }
                PlaneOp::FullScroll => self.stage.plane_a_wave = None,
                PlaneOp::BossArt(round) => {
                    let assets = &mut self.stage.assets;
                    let first = assets.data.uploads.len();
                    assets
                        .data
                        .load_boss_art(&self.rom, usize::from(round))
                        .expect("validated boss art table");
                    for upload in &assets.data.uploads[first..] {
                        let range = upload.first_tile..upload.first_tile + upload.tile_count;
                        assets.tiles[range.clone()].copy_from_slice(&assets.data.tiles[range]);
                    }
                    if round == 7 {
                        assets
                            .data
                            .final_boss_arena(&self.rom)
                            .expect("validated final boss arena");
                    }
                }
                PlaneOp::FinalBossScroll {
                    mode,
                    amplitude,
                    phase,
                } => {
                    self.stage.final_boss_scroll =
                        Some((mode, crate::stage::LineWave { amplitude, phase }));
                }
                _ => {
                    if let Some(raster) = &mut self.stage.boss_raster {
                        // The ops name ROM blocks the boss loaded itself.
                        let _ = raster.apply(&self.rom, op);
                    }
                }
            }
        }
    }
}
