use super::*;

impl Combat {
    /// List 3's shurikens, last allocated first: thrown ones (with their
    /// own priority), spread ones in their `$24EB2` poses and spent ones
    /// (both with the player's, `player_high`). A shuriken that meets a
    /// wall stays the same record, so a spent one keeps its place.
    pub fn draw_player_attacks(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
        player_high: bool,
    ) {
        enum Shuriken {
            Thrown(usize),
            Spread(usize),
            Spent(usize),
        }
        let thrown = self
            .projectiles
            .iter()
            .enumerate()
            .filter(|(_, shot)| shot.alive && !shot.hidden && !shot.enemy)
            .map(|(i, shot)| (shot.order, Shuriken::Thrown(i)));
        let spread = (self.weapons.shots.iter().enumerate())
            .map(|(i, shot)| (shot.order, Shuriken::Spread(i)));
        let spent = (self.weapons.spent.iter().enumerate())
            .map(|(i, spent)| (spent.order, Shuriken::Spent(i)));
        let mut listed: Vec<(u64, Shuriken)> = thrown.chain(spread).chain(spent).collect();
        listed.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        for (_, shuriken) in listed {
            match shuriken {
                Shuriken::Thrown(i) => {
                    let shot = &self.projectiles[i];
                    frame.mark_high_sprites(shot.high_priority);
                    self.draw_projectile(frame, palettes, scene_tiles, camera, shot);
                }
                Shuriken::Spread(i) => {
                    let shot = &self.weapons.shots[i];
                    let (base, left) = match shot.flips {
                        Some(flips) => (flips & 0x1000, flips & 0x800 != 0),
                        None => (0, shot.velocity_x < 0),
                    };
                    frame.mark_high_sprites(player_high);
                    self.draw_object(
                        frame,
                        palettes,
                        scene_tiles,
                        shot.mapping,
                        base,
                        ((shot.x >> 16) - camera.0, (shot.y >> 16) - camera.1),
                        left,
                    );
                }
                Shuriken::Spent(i) => {
                    let spent = &self.weapons.spent[i];
                    frame.mark_high_sprites(player_high);
                    self.draw_object(
                        frame,
                        palettes,
                        scene_tiles,
                        spent.mapping,
                        spent.flip & 0x1000,
                        (spent.x - camera.0, spent.y - camera.1),
                        spent.flip & 0x800 != 0,
                    );
                }
            }
        }
        frame.mark_high_sprites(false);
    }
}
