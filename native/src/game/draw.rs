use super::*;

impl Game {
    /// The rider thrown off in a surf death (`special::thrown`), allocated
    /// at the death, is list 9's last record; its tile word 0 shows the
    /// rider the player's DMA loaded (`PlayerAssets::overlay`).
    pub(super) fn draw_mounted_effects(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        camera: (i32, i32),
        list: SpriteList,
    ) {
        use crate::combat::EffectKind;
        for effect in self.combat.effects.iter().rev().filter(|effect| {
            effect.sprite_list() == list
                && matches!(
                    effect.kind,
                    EffectKind::ThrownRider | EffectKind::HorseWreck | EffectKind::SurfSlashBoard
                )
        }) {
            let pose = if effect.kind == EffectKind::SurfSlashBoard && effect.mapping == 0xb6 {
                Some(&self.sprites.frames[182])
            } else {
                self.sprites.overlay(effect.mapping)
            };
            let Some(pose) = pose else { continue };
            frame.mark_high_sprites(effect.high_priority);
            pose.draw(
                frame,
                palettes,
                effect.x - camera.0,
                effect.y - camera.1,
                false,
            );
            frame.mark_high_sprites(false);
        }
    }
    /// List 2: the player, then the 2-1 horse.
    pub(super) fn draw_player(&self, frame: &mut Frame, palettes: &Lines<'_>, camera: (i32, i32)) {
        let player_high = self.combat.player_high_priority();
        let pose = &self.sprites.frames[self.player.display_frame()];
        let (x, feet) = self.player.position();
        // Ikazuchi charges: every fourth player update shows the shield.
        let ticks = self
            .cast
            .as_ref()
            .map_or(self.player.ticks, |cast| u64::from(cast.fx.frame()));
        let shield = crate::ninjutsu_fx::shield_mapping(
            self.combat.weapons.shield,
            ticks,
            self.player.animator.state,
        );
        // PlayerState_DeathHidden hides the sprite (42, 54); no death state
        // flickers ($21540). The horse death's rider is its list-1 wreck.
        let state = self.player.animator.state;
        let visible = if dying(state) {
            !death_hidden(state)
        } else {
            self.combat.player_visible(state, self.player.ticks)
        };
        frame.mark_high_sprites(player_high);
        // #85: the 2-1 horse is the next list-2 sprite, so it sits behind
        // the rider. It copies the rider's hide bit, so it flickers with it
        // and vanishes in the death, where the unported wreck `$78` takes
        // its place.
        if let Some(horse) = self.special.as_ref().and_then(|special| special.horse)
            && visible
            && !dying(state)
        {
            self.sprites.frames[horse.frame].draw_on_line(
                frame,
                palettes,
                (x - camera.0, feet - camera.1),
                false,
                crate::special::Horse::PALETTE_LINE,
            );
        }
        if visible
            && !shield.is_some_and(|id| {
                self.combat.shield_art.draw(
                    frame,
                    palettes,
                    id,
                    (x - camera.0, feet - camera.1),
                    self.player.facing_left,
                )
            })
        {
            pose.draw(
                frame,
                palettes,
                x - camera.0,
                feet - camera.1,
                self.player.facing_left,
            );
        }
        frame.mark_high_sprites(false);
    }
    pub fn draw(&self, frame: &mut Frame) {
        self.stage.align_frame(frame);
        let (backdrop, index) = self.stage.backdrop();
        frame.clear_backdrop(backdrop, index);
        // $2386 (4-1): below $40 the player's state leaves the line-200
        // H-interrupt on, and no sprite shows below that line.
        let sprite_end = self.scroll[self.part()]
            .sprite_cut
            .filter(|_| self.player.animator.state < 0x40)
            .map(|line| line + 1);
        // Boss4's H-interrupt reloads CRAM line 1 below line 199 (4-3).
        if let Some(change) = self.stage.cram_change() {
            frame.change_cram_line(change);
        }
        // The sprites make up the frame's sprite table (`Objects_BuildSprites`
        // and `$39A02`, see `render::sat`): recorded back to front, then
        // drawn with their explicit masks and priority.
        frame.begin_sprite_table();
        self.stage.scene_game_cut(frame, sprite_end);
        frame.clip_lines(sprite_end);
        // Object_UpdateVisibility subtracts Stage_VScrollOffset (the 7-1
        // bob) from every object's screen Y, as VSRAM adds it to plane A.
        let camera = (
            self.player.camera_x,
            self.player.camera_y + self.stage.raster.vscroll_offset(),
        );
        let lines = self.stage.lines();
        let (palettes, tiles) = (&lines, &self.stage.assets.tiles);
        // `Objects_BuildSprites` walks lists 1 to 9 (`SpriteList`), each
        // record's pieces in link order: painted from list 9 (the 2-1 kites,
        // behind them all, #87) to list 1, each list last linked first.
        for list in SpriteList::BACK_TO_FRONT {
            match list {
                SpriteList::Player => self.draw_player(frame, palettes, camera),
                SpriteList::PlayerAttacks => {
                    self.combat.draw_list(frame, list, palettes, tiles, camera);
                    // The spread and spent shurikens carry the player's
                    // priority bit (`$22C4C` or a boss's raise): high ones
                    // stay over the high plane cells.
                    let player_high = self.combat.player_high_priority();
                    self.combat
                        .draw_player_attacks(frame, palettes, tiles, camera, player_high);
                    if let Some(cast) = &self.cast {
                        cast.fx.draw(frame, palettes, camera);
                    }
                }
                _ => {
                    if matches!(list, SpriteList::Back | SpriteList::Effects) {
                        self.draw_mounted_effects(frame, palettes, camera, list);
                    }
                    self.combat.draw_list(frame, list, palettes, tiles, camera);
                    if list == SpriteList::Pickups
                        && frame.width() > crate::WIDTH
                        && let Some(special) = self.special.as_ref().filter(|s| s.scrolling)
                    {
                        for (mapping, x, y) in self.objects.pickup_approach(
                            self.combat
                                .approaching_pickups(special.camera_x, special.step),
                            camera.0,
                        ) {
                            self.combat.draw_object(
                                frame,
                                palettes,
                                tiles,
                                mapping,
                                0,
                                (x - camera.0, y - camera.1),
                                false,
                            );
                        }
                    }
                    if list == SpriteList::StageObjects
                        && frame.width() > crate::WIDTH
                        && let Some(special) = self.special.as_ref().filter(|s| s.scrolling)
                    {
                        for (mapping, x, y) in self.objects.mounted_approach(
                            special.camera_x,
                            self.player.camera_y,
                            special.step,
                        ) {
                            self.combat.draw_object(
                                frame,
                                palettes,
                                tiles,
                                mapping,
                                0,
                                (x - camera.0, y - camera.1),
                                false,
                            );
                        }
                    }
                    self.objects
                        .draw_list(frame, &self.combat, list, palettes, tiles, camera);
                }
            }
            if list == SpriteList::EnemyShots {
                self.combat.draw_boss_parts(frame, palettes, tiles, camera);
            }
        }
        let under_planes = frame.sprite_count();
        // Priority sprites of the ambient spawners sit over the planes'
        // high-priority cells.
        self.combat.draw_front(frame, palettes, tiles, camera);
        // List 1 heads the sprite table, and the warning sign's tile word
        // is high priority: it covers every other sprite and both planes.
        for marker in self.markers.visible(self.combat.ticks) {
            self.combat.draw_object(
                frame,
                palettes,
                tiles,
                marker.mapping,
                markers::TILE,
                (marker.x - camera.0, marker.y - camera.1),
                false,
            );
        }
        if let Some(special) = &self.special {
            for position in special.warnings.visible() {
                let position = (
                    position.0 + frame.view_right() - crate::WIDTH as i32,
                    position.1,
                );
                self.combat.draw_object(
                    frame,
                    palettes,
                    tiles,
                    crate::special::warning::MAPPING,
                    crate::special::warning::TILE,
                    position,
                    false,
                );
            }
        }
        frame.clip_lines(None);
        hud::window_masks(frame);
        frame.end_sprite_table();
        frame.draw_sprite_table(0..under_planes);
        // Both planes' high-priority cells cover the low-priority sprites.
        frame.behind_high_sprites(true);
        self.stage.front_game(frame);
        frame.behind_high_sprites(false);
        frame.draw_sprite_table(under_planes..frame.sprite_count());
        self.stage.clear_world_padding(frame);
        frame.center_content();
        let hud = &self.combat.hud;
        self.hud_renderer.draw(
            frame,
            HudValues {
                health: hud.health,
                max_health: hud.max_health,
                lives: hud.lives,
                ammo: hud.ammo,
                ninjutsu_uses: self.combat.weapons.ninjutsu_uses,
                ninjutsu_selected: self.combat.weapons.ninjutsu_selected,
                score_display: hud.score_display,
                infinite_shurikens: self.options.infinite,
            },
        );
        if let Some(fade) = self.fade {
            frame.fade(fade);
        }
        // $B04E: a lost life or a part load fades the frame out.
        if let Some(fade) = self.lost_fade.or(self.part_fade) {
            frame.fade(fade);
        }
    }
}
