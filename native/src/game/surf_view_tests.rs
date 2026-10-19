//! Issue #32: the 4-1 surfing enemies enter, leave and are repositioned
//! beyond the visible edges of a widened view, not inside it.
use super::*;
use crate::{combat::EnemyState, rom::word};

fn cartridge(region: &str) -> Vec<u8> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(
        std::fs::read(directory.join(format!("shinobi3_{region}.bin"))).unwrap(),
    )
    .unwrap()
}

/// A drawn surfing enemy: its type, X and world sprite box.
#[derive(Clone, Copy, Debug)]
struct Seen {
    kind: u16,
    routine: u16,
    x: i32,
    area: (i32, i32, i32, i32),
    /// `$70` thruster part (`J_THRUSTER`, `vars[5]`) present.
    thruster: bool,
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn surfing_enemies_never_pop_inside_a_widened_view() {
    let rom = cartridge("us");
    // The rider's wake and thruster parts are drawn at its X.
    let parts = [
        word(&rom, 0x17ad6).unwrap(),
        word(&rom, 0x17ad6).unwrap() - 1,
        word(&rom, 0x1787e).unwrap(),
        word(&rom, 0x17884).unwrap(),
    ];
    let debris = word(&rom, 0x1694c).unwrap();
    let mappings: std::collections::BTreeMap<u16, crate::combat::Mapping> = (0
        ..crate::rom::SPRITE_MAPPING_COUNT)
        .filter_map(|id| Some((id, crate::combat::Mapping::load(&rom, id).ok()?)))
        .collect();
    for width in [426, 560] {
        let mut game = Game::load_stage(&rom, StageId::new(4, 1, 0)).unwrap();
        game.options.invincible = true;
        game.set_view_width(width);
        let mut previous: Vec<Option<Seen>> = Vec::new();
        let mut shots: Vec<((i32, i32), i32)> = Vec::new();
        let mut kinds = std::collections::BTreeSet::new();
        let mut failures = Vec::new();
        for tick in 0..9000 {
            Mode::tick(&mut game, Input::default());
            let combat = &game.combat;
            let camera = (game.player.camera_x, game.player.camera_y);
            let view = combat.screen_view(camera.0);
            let screen = (
                camera.0 + view.0,
                camera.1,
                camera.0 + view.1,
                camera.1 + 224,
            );
            let visible = |(x0, y0, x1, y1): (i32, i32, i32, i32)| {
                x1 > screen.0 && x0 < screen.2 && y1 > screen.1 && y0 < screen.3
            };
            let mut seen = vec![None; combat.enemies.len()];
            for (index, e) in combat.enemies.iter().enumerate() {
                let kind = combat.spawns.get(e.spawn).map_or(0, |spawn| spawn.kind);
                let drawn = match e.state {
                    EnemyState::Dormant | EnemyState::Emerging => false,
                    EnemyState::Defeated => e.dying,
                    _ => true,
                };
                if !matches!(kind, 0x18 | 0x60 | 0x70) || !drawn || e.mapping == 0 {
                    continue;
                }
                kinds.insert(kind);
                let mut frames = vec![e.mapping];
                if kind != 0x18 {
                    frames.extend(parts);
                }
                let span = crate::combat::sprite_span(&mappings, &frames, e.facing_left);
                let (top, bottom) = frames
                    .iter()
                    .filter_map(|id| mappings.get(id))
                    .flat_map(|m| &m.pieces)
                    .fold((0, 0), |(lo, hi), p| {
                        (lo.min(p.y), hi.max(p.y + p.height as i32 * 8))
                    });
                // The wake rides the water line below the rider.
                let bottom = if kind == 0x18 { bottom } else { bottom.max(64) };
                seen[index] = Some(Seen {
                    kind,
                    routine: e.routine,
                    x: e.x,
                    area: (e.x + span.0, e.y + top, e.x + span.1, e.y + bottom),
                    thruster: kind == 0x70 && e.vars[5] != 0,
                });
            }
            for index in 0..seen.len().max(previous.len()) {
                let before = previous.get(index).copied().flatten();
                let now = seen.get(index).copied().flatten();
                let event = match (before, now) {
                    (None, Some(now)) if visible(now.area) => Some(("appeared", now)),
                    (Some(before), None) if visible(before.area) => Some(("vanished", before)),
                    // The guard's drop (routine $10) enters through the top
                    // edge at the original camera + $140 in every view.
                    (Some(before), Some(now)) if before.kind == now.kind => {
                        if (now.x - before.x).abs() > 12
                            && !(now.kind == 0x18 && now.routine == 0x10)
                            && (visible(before.area) || visible(now.area))
                        {
                            Some(("jumped", now))
                        } else if before.thruster && !now.thruster && visible(now.area) {
                            Some(("lost its thruster", now))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some((what, at)) = event {
                    failures.push(format!(
                        "{width} px tick {tick}: ${:02X} routine ${:02X} {what} at {:?}, screen {:?}",
                        at.kind, at.routine, at.area, screen
                    ));
                }
            }
            // Floating debris enters beyond the right edge and is freed
            // once past the left one (contact with the player aside).
            // Each with where its next step takes it.
            let now: Vec<((i32, i32), i32)> = combat
                .projectiles
                .iter()
                .filter(|shot| {
                    shot.alive
                        && shot.owner.is_some_and(|owner| {
                            combat.enemies.get(owner).is_some_and(|e| {
                                combat.spawns.get(e.spawn).is_some_and(|s| s.kind == 0x3c)
                            })
                        })
                })
                .map(|shot| {
                    let step = shot.velocity_x + (shot.motion.vx >> 16);
                    ((shot.x, shot.y), shot.x + step)
                })
                .collect();
            let span = crate::combat::sprite_span(&mappings, &[debris, debris + 1], false);
            let area = |(x, y): (i32, i32)| (x + span.0, y - 16, x + span.1, y + 16);
            let near = |a: &(i32, i32), b: &(i32, i32)| (a.0 - b.0).abs() <= 8 && a.1 == b.1;
            for (at, _) in &now {
                if !shots.iter().any(|(before, _)| near(before, at)) && visible(area(*at)) {
                    failures.push(format!("{width} px tick {tick}: debris appeared at {at:?}"));
                }
            }
            for &(at, next) in &shots {
                let player = game.player.position().0;
                if !now.iter().any(|(after, _)| near(&at, after))
                    && at.0 < player - 48
                    && visible(area((next, at.1)))
                {
                    failures.push(format!("{width} px tick {tick}: debris vanished at {at:?}"));
                }
            }
            shots = now;
            previous = seen;
        }
        assert!(
            kinds.contains(&0x60) && kinds.contains(&0x70),
            "{width} px: surfing riders never appeared ({kinds:?})"
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
