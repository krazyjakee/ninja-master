use super::*;

fn cartridge(region: &str) -> Vec<u8> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(
        std::fs::read(directory.join(format!("shinobi3_{region}.bin"))).unwrap(),
    )
    .unwrap()
}

struct Floor;
impl Terrain for Floor {
    fn width(&self) -> i32 {
        4096
    }
    fn solid_at(&self, _: i32, y: i32) -> bool {
        y >= 496
    }
}

#[test]
#[ignore = "requires the three user-supplied cartridges"]
fn cave_boss_damage_gates_match_original_rom() {
    for region in ["us", "eu", "jp"] {
        let rom = cartridge(region);
        let mut base = Combat::load_stage(&rom, crate::stage::StageId::new(1, 3, 0)).unwrap();
        base.set_invincible(true);
        base.tick((100, 496), false, (0, 288), &Floor);
        let index = base
            .enemies
            .iter()
            .position(|e| base.spawns[e.spawn].kind == crate::bosses::BOSS_KIND)
            .unwrap();
        for row in include_str!("../../tests/data/cave_boss_hits.csv")
            .lines()
            .skip(1)
        {
            let v: Vec<u16> = row.split(',').map(|n| n.parse().unwrap()).collect();
            let mut combat = base.clone();
            combat.set_player_pose(usize::from(v[2]), 0, 1, false);
            let enemy = &mut combat.enemies[index];
            enemy.routine = v[0];
            enemy.phase = v[1] as u8;
            enemy.x = 160;
            enemy.y = 496;
            enemy.mapping = 0x23d;
            enemy.timer = 15;
            enemy.counter = 33;
            enemy.vars[2] = 40; // stance wait ($34)
            enemy.vars[3] = 50; // stance patience ($36)
            combat.tick((100, 496), v[3] != 0, (0, 288), &Floor);
            if v[4] >= 0x50 {
                // Exercise the real magic entry point, including its target filter.
                combat.strike_visible((0, 288), 1);
            } else if v[4] == 0 {
                // A real stationary shuriken at the original probe's
                // overlap point exercises collision and weapon spending.
                combat
                    .projectiles
                    .push(Projectile::new(160, 456, 0, false, 0xca, 1));
                let mut events = CombatEvents::default();
                combat.run_attacks((100, 496), v[3] != 0, (0, 288), &Floor, &mut events);
            } else {
                let damage = if v[4] == 0x4c { 3 } else { 1 };
                let enemy = &mut combat.enemies[index];
                Combat::hurt_enemy(
                    &combat.roster,
                    enemy,
                    &combat.spawns[enemy.spawn],
                    damage,
                    true,
                );
            }
            assert_eq!(12 - combat.enemies[index].health, v[5], "{region}: {row}");
        }
    }
}

#[test]
#[ignore = "requires the three user-supplied cartridges"]
fn cave_boss_accepts_a_real_frontal_dash_kick_in_stance() {
    for region in ["us", "eu", "jp"] {
        let rom = cartridge(region);
        let mut combat = Combat::load_stage(&rom, crate::stage::StageId::new(1, 3, 0)).unwrap();
        combat.set_invincible(true);
        combat.tick((120, 496), false, (0, 288), &Floor);
        let index = combat
            .enemies
            .iter()
            .position(|e| combat.spawns[e.spawn].kind == crate::bosses::BOSS_KIND)
            .unwrap();
        let enemy = &mut combat.enemies[index];
        enemy.routine = 0x18;
        enemy.x = 160;
        enemy.mapping = 0x23d;
        enemy.vars[2] = 40;
        combat.set_player_pose(movement::state::DASH_KICK, 0, 1, false);
        combat.start_player_attack(movement::AttackPosture::DashKick, (120, 496), false);
        for _ in 0..30 {
            combat.tick((120, 496), false, (0, 288), &Floor);
            if combat.enemies[index].health != 12 {
                break;
            }
        }
        assert_eq!(
            combat.enemies[index].health, 9,
            "{region}: dash kick was guarded"
        );
    }
}
