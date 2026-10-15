use super::*;

impl Combat {
    /// The records of sprite list `list` this module holds, painted last
    /// linked first so that the first record wins overlaps
    /// (`Objects_BuildSprites`, see [`SpriteList`]). The player's shurikens
    /// are [`Self::draw_player_attacks`]; Boss4's list-1 parts are
    /// [`Self::draw_boss_parts`].
    pub fn draw_list(
        &self,
        frame: &mut Frame,
        list: SpriteList,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
    ) {
        match list {
            SpriteList::Enemies | SpriteList::Bosses => {}
            // List 4 holds the enemies' shots (the player's are
            // `draw_player_attacks`). A shot that expired touching the body
            // waits for the damage check.
            SpriteList::EnemyShots => {
                for shot in self
                    .projectiles
                    .iter()
                    .rev()
                    .filter(|shot| shot.alive && !shot.hidden && !shot.offscreen && shot.enemy)
                {
                    frame.mark_high_sprites(shot.high_priority);
                    self.draw_projectile(frame, palettes, scene_tiles, camera, shot);
                }
                frame.mark_high_sprites(false);
            }
            _ => {}
        }
        self.draw_effects(frame, list, palettes, scene_tiles, camera);
        self.draw_enemies(frame, palettes, scene_tiles, camera, list);
    }
    /// The effect records of `list` in reverse link order: the ones linked
    /// at the list's tail newest first, then the ones linked at its head
    /// ([`Effect::head`]) oldest first. The ambient spawners' priority
    /// sprites are [`Self::draw_front`] and Boss4's list-1 parts
    /// [`Self::draw_boss_parts`].
    pub(super) fn draw_effects(
        &self,
        frame: &mut Frame,
        list: SpriteList,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
    ) {
        let listed = |effect: &&Effect| {
            effect.sprite_list() == list
                && !effect.in_front()
                && !(list == SpriteList::Effects && effect.kind == EffectKind::BossPart)
                // Drawn on the player's patterns (`Game::draw`).
                && !matches!(
                    effect.kind,
                    EffectKind::ThrownRider | EffectKind::HorseWreck | EffectKind::SurfSlashBoard
                )
        };
        let tail = self.effects.iter().rev().filter(|e| !e.head).filter(listed);
        let head = self.effects.iter().filter(|e| e.head).filter(listed);
        for effect in tail.chain(head) {
            frame.mark_high_sprites(effect.high_priority);
            self.draw_effect(frame, palettes, scene_tiles, camera, effect);
        }
        frame.mark_high_sprites(false);
    }
    /// Boss4's list-1 parts (`bosses::boss4::parts`), newest first. They
    /// keep the place they had before the lists were drawn in order, after
    /// list 4 (4-3 is being worked on separately).
    pub fn draw_boss_parts(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
    ) {
        for effect in self.effects.iter().rev().filter(|effect| {
            effect.kind == EffectKind::BossPart && effect.sprite_list() == SpriteList::Effects
        }) {
            frame.mark_high_sprites(effect.high_priority);
            self.draw_effect(frame, palettes, scene_tiles, camera, effect);
        }
        frame.mark_high_sprites(false);
    }
    pub(super) fn draw_projectile(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
        shot: &Projectile,
    ) {
        // Enemy weapons may use their thrower's graphics base; art that
        // faces left (the sickle) mirrors while moving right.
        let left = shot.leftward() != shot.faces_left;
        self.draw_object(
            frame,
            palettes,
            scene_tiles,
            shot.mapping,
            shot.base,
            (shot.x - camera.0, shot.y - camera.1),
            left,
        );
    }
    pub(super) fn draw_effect(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
        effect: &Effect,
    ) {
        if effect.delay > 0
            || effect.flags & 0x880 != 0
            || effect.mapping == 0
            || (effect.blink && !effect.age.is_multiple_of(2))
            || !self.mappings.contains_key(&effect.mapping)
        {
            return;
        }
        self.draw_mapping(
            frame,
            palettes,
            scene_tiles,
            effect.mapping,
            effect.base,
            (effect.x - camera.0, effect.y - camera.1),
            effect.flip,
            // Both sequences use $4D8..$4F9, whose contents vary by stage.
            matches!(effect.kind, EffectKind::Puff | EffectKind::Debris),
        );
    }
    /// Indices of the enemies of `list`, last allocated first: painted in
    /// this order, the first record wins overlaps.
    pub(crate) fn paint_order(&self, list: SpriteList) -> impl Iterator<Item = usize> + use<> {
        let mut listed: Vec<(u64, usize)> = self
            .enemies
            .iter()
            .enumerate()
            .filter(|(_, enemy)| self.enemy_list(enemy) == list)
            .map(|(index, enemy)| (enemy.order, index))
            .collect();
        listed.sort_unstable_by(|a, b| b.cmp(a));
        listed.into_iter().map(|(_, index)| index)
    }
    /// The enemies of `list` in reverse list order (the first record wins
    /// overlaps). Freed slots are reused, so list order is the allocation
    /// order, not the index.
    pub(super) fn draw_enemies(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
        list: SpriteList,
    ) {
        for enemy in self.paint_order(list).map(|index| &self.enemies[index]) {
            let visible = match enemy.state {
                EnemyState::Dormant | EnemyState::Emerging => false,
                EnemyState::Defeated => enemy.dying,
                _ => true,
            };
            if !visible {
                continue;
            }
            let kind = self.spawns[enemy.spawn].kind;
            if enemy.mapping != 0 {
                let mut actor_palettes = *palettes;
                let mut flashed = *palettes[1].colors();
                // The samurai write CRAM line 1 themselves (`PaletteOp`).
                // Natively the flash is the actor's own copy of the line.
                if enemy.flash > 0 && !matches!(kind, 0xb4 | 0xb8) {
                    self.flash_palette(&mut flashed);
                    actor_palettes[1] = PaletteRef::Fixed(&flashed);
                }
                let base = enemy
                    .tile
                    .or_else(|| self.bases.get(&kind).copied())
                    .unwrap_or(0x2300);
                frame.mark_high_sprites(enemy.high_priority);
                self.draw_object(
                    frame,
                    &actor_palettes,
                    scene_tiles,
                    enemy.mapping,
                    base,
                    (enemy.x - camera.0, enemy.y - camera.1),
                    enemy.facing_left,
                );
                frame.mark_high_sprites(false);
            }
            // Issue #51: an unported type (or a documented stand-in) is
            // outlined instead of passing off a guessed pose as recovered.
            if self.roster.marked(kind) {
                let area = enemies::marker_box(&self.mappings, enemy);
                Self::draw_outline(frame, area, camera, UNPORTED_OUTLINE);
            }
        }
    }
    /// The effects `Effect::in_front` keeps out of `draw`: the game paints
    /// them after the high-priority plane.
    pub fn draw_front(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
    ) {
        for effect in self.effects.iter().filter(|effect| effect.in_front()) {
            self.draw_object(
                frame,
                palettes,
                scene_tiles,
                effect.mapping,
                effect.base,
                (effect.x - camera.0, effect.y - camera.1),
                effect.flip,
            );
        }
    }
    /// `$1BEC4`: the white hit flash over colours 0..7 of an actor's line.
    pub(super) fn flash_palette(&self, line: &mut Palette) {
        line[..8].fill(md_color(self.rules.flash_color));
    }
    /// One-pixel outline of `area` (world coordinates).
    pub(super) fn draw_outline(
        frame: &mut Frame,
        area: Hitbox,
        camera: (i32, i32),
        color: [u8; 3],
    ) {
        let left = area.x - area.half_width - camera.0;
        let top = area.y - area.half_height - camera.1;
        let (width, height) = (area.half_width * 2, area.half_height * 2);
        frame.overlay_rect(left, top, width, 1, color);
        frame.overlay_rect(left, top + height - 1, width, 1, color);
        frame.overlay_rect(left, top, 1, height, color);
        frame.overlay_rect(left + width - 1, top, 1, height, color);
    }
    /// Draw mapping `id` with attribute base `base` at screen `position`,
    /// from the resident art first (Joe's powered patterns while powered),
    /// then the scene's tiles.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_object(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        id: u16,
        base: u16,
        position: (i32, i32),
        left: bool,
    ) {
        self.draw_mapping(
            frame,
            palettes,
            scene_tiles,
            id,
            base,
            position,
            left,
            false,
        );
    }
    /// [`Self::draw_object`]; `puffs` selects the separate explosion art,
    /// including in arenas where the scene uses those tile indices for bosses.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_mapping(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        id: u16,
        base: u16,
        position: (i32, i32),
        left: bool,
        puffs: bool,
    ) {
        let Some(mapping) = self.mappings.get(&id) else {
            return;
        };

        draw_pieces(
            frame,
            &mapping.pieces,
            palettes,
            base,
            position,
            left,
            crate::render::SatWriter::Object,
            |index| {
                self.weapons
                    .powered
                    .then(|| self.powered_tiles.get(&index))
                    .flatten()
                    .or_else(|| puffs.then(|| self.puff_tiles.get(&index)).flatten())
                    .or_else(|| self.common_tiles.get(&index))
                    .or_else(|| scene_tiles.get(index))
            },
        );
    }
}
