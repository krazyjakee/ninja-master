//! Boss4's hull and weapon parts collide apart (`$2DA58`, `$2FD1E`): a
//! shuriken over the hull gets the deflection flag (`$2DB10`) and its next
//! update rebounds through `Func_024AFE` (issue #33).
use super::*;
use crate::combat::{PartTarget, Reaction, WeaponContact};
use crate::enemies::HitResponse;

const PART_C: u8 = 1;
const HULL: u8 = 3;
const COOL: usize = 6;
const OPEN: usize = 11;
const AT: (i32, i32) = (200, 400);
const CAMERA: (i32, i32) = (0, 288);
const PLAYER: (i32, i32) = (100, 496);

fn cartridge() -> Vec<u8> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(std::fs::read(directory.join("shinobi3_us.bin")).unwrap()).unwrap()
}

struct Sky;
impl Terrain for Sky {
    fn width(&self) -> i32 {
        4096
    }
    fn solid_at(&self, _: i32, _: i32) -> bool {
        false
    }
}

fn hull_box() -> Hitbox {
    Hitbox {
        x: 0,
        y: 0,
        half_width: 48,
        half_height: 24,
    }
}

fn part_box(dy: i32) -> Hitbox {
    Hitbox {
        x: 0,
        y: dy,
        half_width: 8,
        half_height: 8,
    }
}

/// The flying machine alone, its weak part `dy` below the hull's centre
/// taking hits, and the hull as its armor.
fn machine(dy: i32) -> Combat {
    let rom = cartridge();
    let mut combat = Combat::load_stage(&rom, crate::stage::StageId::new(4, 3, 0)).unwrap();
    combat
        .enemies
        .retain(|e| combat.spawns[e.spawn].kind == crate::bosses::BOSS_KIND);
    assert_eq!(combat.enemies.len(), 1);
    let boss = &mut combat.enemies[0];
    boss.state = EnemyState::Waiting;
    boss.routine = 0x34;
    boss.phase = PART_C;
    (boss.x, boss.y) = AT;
    boss.health = 0x4000;
    boss.body = Some(part_box(dy));
    boss.armor = Some(hull_box());
    boss.vars[OPEN] = 1;
    boss.vars[COOL] = 0;
    combat
}

fn throw(combat: &mut Combat, x: i32, y: i32, speed: i32) {
    let mut shot = Projectile::new(x - speed, y, speed, false, 0xca, 1);
    shot.order = combat.allocate();
    combat.projectiles.push(shot);
}

fn attacks(combat: &mut Combat) -> CombatEvents {
    let mut events = CombatEvents::default();
    combat.run_attacks(PLAYER, false, CAMERA, &Sky, &mut events);
    events
}

fn shot(combat: &Combat) -> Option<&Projectile> {
    combat.projectiles.iter().find(|s| !s.enemy && s.alive)
}

fn cue(combat: &Combat, event: SoundEvent) -> u8 {
    combat.rules.cues.get(event)
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn weak_point_hit_damages_the_part_and_spends_the_shuriken() {
    // The part hangs below the hull: a shot into it meets no armor.
    let mut combat = machine(48);
    throw(&mut combat, AT.0, AT.1 + 48, 8);
    let events = attacks(&mut combat);
    assert!(shot(&combat).is_none());
    assert!(combat.weapons.spent.is_empty());
    assert!(combat.enemies[0].health < 0x4000);
    assert_eq!(combat.enemies[0].reaction, Some(Reaction::Hurt));
    assert!(
        events
            .sound_ids
            .contains(&cue(&combat, SoundEvent::ShurikenHit))
    );
    assert!(
        !events
            .sound_ids
            .contains(&cue(&combat, SoundEvent::ShurikenBounce))
    );
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn hull_hit_beside_the_weak_point_rebounds_without_damage() {
    let mut combat = machine(48);
    throw(&mut combat, AT.0 + 32, AT.1, 8);
    let events = attacks(&mut combat);
    // Flagged where the step ended; nothing is damaged or spent yet.
    let flagged = shot(&combat).expect("the deflected shot waits for its update");
    assert!(flagged.deflected);
    assert_eq!((flagged.x, flagged.y), (AT.0 + 32, AT.1));
    assert_eq!(combat.enemies[0].health, 0x4000);
    assert_eq!(combat.enemies[0].reaction, None);
    assert!(events.sound_ids.is_empty());
    // Its next update: Func_024AFE knocks it back (US ROM probe: +8 px
    // becomes -2 px, -4 px up, 32 ticks, counted once that update).
    let events = attacks(&mut combat);
    assert!(shot(&combat).is_none());
    assert_eq!(combat.weapons.spent.len(), 1);
    let spent = &combat.weapons.spent[0];
    assert_eq!((spent.x, spent.y), (AT.0 + 32, AT.1));
    assert_eq!(spent.motion.vx, -2 << 16);
    assert_eq!(spent.motion.vy, -4 << 16);
    assert_eq!(spent.life, 31);
    assert_eq!(
        events.sound_ids,
        vec![cue(&combat, SoundEvent::ShurikenBounce)]
    );
    assert_eq!(combat.enemies[0].health, 0x4000);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn leftward_and_rising_rebounds_match_the_rom_probe() {
    let mut combat = machine(48);
    let mut rising = Projectile::new(AT.0 - 24, AT.1, -8, false, 0xca, 1);
    rising.motion.vy = -2 << 16;
    rising.order = combat.allocate();
    combat.projectiles.push(rising);
    attacks(&mut combat);
    attacks(&mut combat);
    let spent = &combat.weapons.spent[0];
    assert_eq!(spent.motion.vx, 2 << 16);
    assert_eq!(spent.motion.vy, 0);
    assert_eq!(spent.life, 23);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn powered_shot_is_removed_silently_by_the_hull() {
    let mut combat = machine(48);
    combat.weapons.powered = true;
    throw(&mut combat, AT.0 + 32, AT.1, 8);
    attacks(&mut combat);
    let events = attacks(&mut combat);
    assert!(shot(&combat).is_none());
    assert!(combat.weapons.spent.is_empty());
    assert!(events.sound_ids.is_empty());
    assert_eq!(combat.enemies[0].health, 0x4000);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn genuine_miss_keeps_flying() {
    let mut combat = machine(48);
    throw(&mut combat, AT.0 + 120, AT.1, 8);
    attacks(&mut combat);
    let flying = shot(&combat).unwrap();
    assert!(!flying.deflected);
    assert_eq!(flying.x, AT.0 + 120);
    assert!(combat.weapons.spent.is_empty());
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn overlapping_hull_and_weak_point_damage_with_the_rebound_sound() {
    // Immediate: the hull's flag first, then the part's hit; the update
    // takes Func_024AFE's sound and PlayerAttack_Run frees it.
    let mut combat = machine(0);
    throw(&mut combat, AT.0, AT.1, 8);
    let events = attacks(&mut combat);
    assert!(shot(&combat).is_none());
    assert!(combat.weapons.spent.is_empty());
    assert!(combat.enemies[0].health < 0x4000);
    assert!(
        events
            .sound_ids
            .contains(&cue(&combat, SoundEvent::ShurikenBounce))
    );
    assert!(
        !events
            .sound_ids
            .contains(&cue(&combat, SoundEvent::ShurikenHit))
    );

    // Deferred: the hit is held and lands on the next object pass.
    let mut combat = machine(0);
    combat.deferring = true;
    throw(&mut combat, AT.0, AT.1, 8);
    attacks(&mut combat);
    let held = shot(&combat).unwrap();
    assert!(held.deflected && held.held.is_some());
    let mut events = CombatEvents::default();
    combat.land_strikes(&mut events);
    assert!(shot(&combat).is_none());
    assert!(combat.weapons.spent.is_empty());
    assert!(combat.enemies[0].health < 0x4000);
    assert!(
        events
            .sound_ids
            .contains(&cue(&combat, SoundEvent::ShurikenBounce))
    );
    assert!(
        !events
            .sound_ids
            .contains(&cue(&combat, SoundEvent::ShurikenHit))
    );
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn part_immunity_keeps_the_hull_deflecting() {
    let mut combat = machine(48);
    combat.enemies[0].vars[COOL] = 4;
    throw(&mut combat, AT.0 + 32, AT.1, 8);
    attacks(&mut combat);
    assert!(shot(&combat).unwrap().deflected);
    // The hull collides once per object update.
    let mut combat = machine(48);
    throw(&mut combat, AT.0 + 32, AT.1, 8);
    throw(&mut combat, AT.0 - 32, AT.1, 8);
    attacks(&mut combat);
    let flags: Vec<bool> = combat
        .projectiles
        .iter()
        .filter(|s| !s.enemy && s.alive)
        .map(|s| s.deflected)
        .collect();
    assert_eq!(flags, vec![true, false]);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn spread_shuriken_rebounds_off_the_hull() {
    let mut combat = machine(48);
    let order = combat.allocate();
    combat.weapons.shots.push(SpreadShot {
        x: (AT.0 + 24) << 16,
        y: AT.1 << 16,
        velocity_x: 8 << 16,
        velocity_y: 0,
        mapping: 0xca,
        high_priority: false,
        age: 0,
        in_water: false,
        damage: 1,
        direction: 0,
        turn: None,
        flips: None,
        held: None,
        untested: false,
        deflected: false,
        facing_left: false,
        order,
    });
    attacks(&mut combat);
    assert!(combat.weapons.shots[0].deflected);
    let events = attacks(&mut combat);
    assert!(combat.weapons.shots.is_empty());
    assert_eq!(combat.weapons.spent.len(), 1);
    assert_eq!(combat.weapons.spent[0].motion.vx, -2 << 16);
    assert!(
        events
            .sound_ids
            .contains(&cue(&combat, SoundEvent::ShurikenBounce))
    );
    assert_eq!(combat.enemies[0].health, 0x4000);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn armored_part_contact_flags_the_shuriken_and_leaves_it_to_later_parts() {
    let mut combat = machine(48);
    combat.enemies[0].armor = None;
    throw(&mut combat, AT.0 + 120, AT.1, 0);
    let area = Hitbox {
        x: AT.0 + 120,
        y: AT.1,
        half_width: 8,
        half_height: 8,
    };
    let contact = WeaponContact {
        area,
        target: PartTarget::Deflect,
        high_priority: false,
    };
    combat.weapon_contacts = vec![contact, contact];
    let mut events = CombatEvents::default();
    combat.hit_weapon_parts(&mut events);
    let flagged = shot(&combat).unwrap();
    assert!(flagged.deflected);
    assert!(events.sound_ids.is_empty());
    attacks(&mut combat);
    assert_eq!(combat.weapons.spent.len(), 1);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn hull_vulnerability_and_non_collidable_states_are_preserved() {
    let mut combat = machine(48);
    let kind = crate::bosses::BOSS_KIND;
    let response = |combat: &Combat| combat.roster.hit(kind, &combat.enemies[0]);
    // A closed weak part: the whole hull rebounds shots.
    combat.enemies[0].vars[OPEN] = 0;
    assert_eq!(response(&combat), HitResponse::Rebound);
    // The final stage: the hull is the body and takes the hits.
    {
        let boss = &mut combat.enemies[0];
        boss.phase = HULL;
        boss.armor = None;
        boss.body = Some(hull_box());
        boss.step_x = 9;
        boss.health = 10;
    }
    assert_eq!(response(&combat), HitResponse::Accept);
    throw(&mut combat, AT.0 + 32, AT.1, 8);
    let events = attacks(&mut combat);
    assert!(shot(&combat).is_none());
    assert!(combat.weapons.spent.is_empty());
    assert!(combat.enemies[0].health < 10);
    assert!(
        events
            .sound_ids
            .contains(&cue(&combat, SoundEvent::ShurikenHit))
    );
    // Its hit immunity spends shots without a rebound.
    combat.enemies[0].vars[COOL] = 4;
    assert_eq!(response(&combat), HitResponse::Ignore);
    // Routine $30 rebounds once its immunity is over.
    combat.enemies[0].phase = 0;
    combat.enemies[0].routine = 0x30;
    assert_eq!(response(&combat), HitResponse::Ignore);
    combat.enemies[0].vars[COOL] = 0;
    assert_eq!(response(&combat), HitResponse::Rebound);
    // The intro, the sinking gunboat and the death sequence: the armor
    // does not collide and shots fly on.
    for routine in [0, 0x2c, 0x84] {
        let mut combat = machine(48);
        combat.enemies[0].routine = routine;
        assert_eq!(response(&combat), HitResponse::Pass);
        throw(&mut combat, AT.0 + 32, AT.1, 8);
        attacks(&mut combat);
        assert!(!shot(&combat).unwrap().deflected, "routine ${routine:x}");
    }
}
