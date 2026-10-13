//! The borrowed state a type's handler works through for one update pass.
use super::{Child, LIST4_CAPACITY, LIST5_CAPACITY, Shared, Snapshot, frame_at, list4_has_space};
use crate::{
    combat::{
        Blast, Effect, EffectKind, Enemy, EnemyState, Mapping, Motion, PaletteOp, PlaneOp,
        Projectile, Reaction, Signal, SpriteList,
    },
    game::Terrain,
    movement::PlayerEvent,
};
use std::collections::BTreeMap;

/// Borrowed state for one enemy/effect update pass.
pub(crate) struct Ctx<'a> {
    pub(crate) weapon_contacts: &'a mut Vec<crate::combat::WeaponContact>,
    pub(crate) shared: &'a Shared,
    pub(crate) mappings: &'a BTreeMap<u16, Mapping>,
    /// `Object_GfxBases` of the part's object groups, by type.
    pub(crate) bases: &'a BTreeMap<u16, u16>,
    pub(crate) effects: &'a mut Vec<Effect>,
    pub(crate) projectiles: &'a mut Vec<Projectile>,
    pub(crate) projectile_scratch: &'a mut Vec<u16>,
    pub(crate) random: &'a mut u16,
    pub(crate) sounds: &'a mut Vec<u8>,
    /// Real HUD score (`Score`), credited by the death routine rather than
    /// the fatal weapon contact.
    pub(crate) score: &'a mut u32,
    /// Player `+$2B` bit 3: scripted movement suppresses `Func_0129C0`.
    pub(crate) score_suppressed: bool,
    /// `Game_Options` bits 2-3: 0 NORMAL, 1 EASY, 2 HARD, 3 EXPERT.
    pub(crate) level: u8,
    /// Objects allocated this pass and not yet linked into the list.
    pub(crate) children: Vec<Child>,
    /// Every object as it was before this pass, by `Combat::enemies` index;
    /// for the records allocated during the pass, as the pass left them.
    pub(crate) others: Vec<Snapshot>,
    /// Live list-5 objects before this pass, and those adopted since.
    pub(crate) live: usize,
    /// Positions and sprite priorities of the player's list-3 attack objects
    /// (thrown and spread shurikens, the active sword object), scanned by
    /// `Func_012BFE`.
    pub(crate) attacks: &'a [(i32, i32, bool)],
    pub(crate) player: (i32, i32),
    pub(crate) player_input: crate::input::Input,
    pub(crate) player_state: Option<usize>,
    /// Player `plr_ctrl` bit 5, independent of height or animation state.
    pub(crate) player_airborne: bool,
    pub(crate) player_facing_left: bool,
    pub(crate) camera: (i32, i32),
    /// Visible X range relative to `camera`, including widescreen edges.
    pub(crate) view: (i32, i32),
    pub(crate) camera_locked: bool,
    pub(crate) mech_hand_contact: bool,
    pub(crate) star_damage: u8,
    pub(crate) throw_sound: u8,
    /// Index of the object being updated.
    pub(crate) current: usize,
    /// `obj_spawn_flags` of the object being updated (its record's flags).
    pub(crate) spawn_flags: u16,
    /// What the handlers asked of the game this pass, in order.
    pub(crate) signals: Vec<Signal>,
    /// Last write to the player's `plr_event` byte in object-list order.
    pub(crate) player_event: Option<PlayerEvent>,
    /// The owner of an enemy shot that landed on the player last tick
    /// (`Combat::landed_shot`).
    pub(crate) landed_shot: Option<usize>,
    /// `$FF3D05`/`$FF3D06`, the 3-1 `$FC` counters.
    pub(crate) mutants: &'a mut super::round3::Mutants,
    /// The player's box as `Object_CollidePlayer` meets it: its frame's
    /// hitbox at [`Self::player_at`]; `None` while it is hidden (the
    /// immunity blink) or on the raised priority.
    pub(crate) player_body: Option<crate::combat::Hitbox>,
    /// The player's `obj_x`/`obj_y` during the pass: where its update left
    /// it before `Object_Integrate` adds the velocity.
    pub(crate) player_at: (i32, i32),
    /// Damage written to the player's `obj_health` byte by a handler's own
    /// `Object_CollidePlayer` (the player's bit 14 is set).
    pub(crate) player_contact: Option<u8>,
    /// `Player_ForceSurfJump`'s drift ([`crate::combat::CombatEvents::surf_jump`]).
    pub(crate) surf_jump: Option<i32>,
}
/// [`Ctx::explode`] for callers outside the update pass (a list-4 object
/// that meets the player, `Func_01776E`).
pub(crate) fn explode(
    shared: &Shared,
    random: &mut u16,
    effects: &mut Vec<Effect>,
    sounds: &mut Vec<u8>,
    (x, y): (i32, i32),
    blast: Blast,
    high_priority: bool,
) {
    let mut next = || next_random(random);
    for i in 0..shared.puffs {
        let dx = i32::from(next() & 31) - 16;
        let dy = -i32::from(next() & 63);
        let angle = shared.puff_angle + (next() & shared.puff_spread);
        let (cos, sin) = shared.sine.polar(angle);
        let mut puff = Effect::new(EffectKind::Puff, x + dx, y + dy, 0);
        // $36DBE/$36E7C -> $12DD8: controller and puffs inherit bit 15.
        puff.high_priority = high_priority;
        puff.base = if high_priority { 0x8000 } else { 0 };
        // The controller allocates one per update, from its first.
        puff.delay = i + 1;
        puff.motion = Motion {
            vx: cos << blast.x_shift,
            vy: sin << blast.y_shift,
            gravity: shared.puff_gravity,
            ..Motion::default()
        };
        effects.push(puff);
    }
    sounds.push(blast.sound);
}

/// `Func_036FF2` for callers inside or outside the update pass (see
/// [`SmallBlast`](super::shared::SmallBlast)).
pub(crate) fn small_blast(
    shared: &Shared,
    random: &mut u16,
    effects: &mut Vec<Effect>,
    sounds: &mut Vec<u8>,
    camera: (i32, i32),
    (x, y): (i32, i32),
) {
    let blast = shared.small_blast;
    if crate::combat::in_sound_window((x, y), camera)
        && let Some(command) = shared.effects.command(blast.sound)
    {
        sounds.push(command);
    }
    for _ in 0..blast.count {
        let angle = next_random(random) & blast.mask;
        let (cos, sin) = shared.sine.polar(angle);
        let mut puff = Effect::new(EffectKind::Debris, x, y, blast.first);
        puff.motion = Motion {
            vx: cos << 1,
            vy: sin << 1,
            ..Motion::default()
        };
        effects.push(puff);
    }
}

impl Ctx<'_> {
    /// Keep screen-edge entrances beyond the expanded view, allowing for
    /// the whole sprite rather than just its origin. Original-width play
    /// retains the cartridge's entry coordinates.
    pub(crate) fn entry_x(&self, offset: i32, frames: &[u16], left: bool) -> i32 {
        if self.view.1 - self.view.0 == crate::WIDTH as i32 {
            return self.camera.0 + offset;
        }
        let mut first = 0;
        let mut last = 0;
        for piece in frames
            .iter()
            .filter_map(|id| self.mappings.get(id))
            .flat_map(|m| &m.pieces)
        {
            let (lo, hi) = (piece.x, piece.x + piece.width as i32 * 8);
            let (lo, hi) = if left { (-hi, -lo) } else { (lo, hi) };
            first = first.min(lo);
            last = last.max(hi);
        }
        self.camera.0
            + if offset < 0 {
                self.view.0 + offset.min(-last - 8)
            } else {
                self.view.1 + (offset - crate::WIDTH as i32).max(-first + 8)
            }
    }

    /// `Func_0129C0`: credit the points of the lethal hit, unless the
    /// scripted player routine suppresses them.
    pub(super) fn award_pending_score(&mut self, enemy: &mut Enemy) {
        let points = std::mem::take(&mut enemy.pending_score);
        if !self.score_suppressed {
            *self.score = self.score.saturating_add(points);
        }
    }
    /// `obj_tile` bit 15 of the object being updated.
    pub(crate) fn current_high_priority(&self) -> bool {
        self.others
            .get(self.current)
            .is_some_and(|enemy| enemy.high_priority)
    }
    pub(crate) fn player_event(&mut self, event: PlayerEvent) {
        self.player_event = Some(event);
    }
    /// `Object_CollidePlayer` (`$1942`) with `hitbox`: the strict overlap
    /// with [`Self::player_body`], unless an earlier collision this pass
    /// already set the player's bit 14.
    pub(crate) fn collide_player(&self, hitbox: crate::combat::Hitbox) -> bool {
        self.player_contact.is_none() && self.player_body.is_some_and(|body| body.collides(hitbox))
    }
    /// `move.b #damage,7(a3)` after [`Self::collide_player`]: the contact
    /// the next `Player_CheckDamage` takes.
    pub(crate) fn contact_player(&mut self, damage: u8) {
        self.player_contact = Some(damage);
    }
    /// `Player_ForceSurfJump` (`$2292C`): `PS_SurfJump` at cursor `$100`,
    /// `plr_drift` = the `$FF001C` low byte (not advanced) >> 6.
    pub(crate) fn force_surf_jump(&mut self) {
        self.surf_jump = Some(i32::from(*self.random as u8 as i8) >> 6);
    }
    /// Ask the game for `signal` once this pass ends.
    pub(crate) fn signal(&mut self, signal: Signal) {
        self.signals.push(signal);
    }
    /// `$FF0029` bit 1: the round clear / next substage.
    pub(crate) fn next_substage(&mut self) {
        self.signal(Signal::NextSubstage);
    }
    /// `Func_00342E`: hold the player on screen.
    pub(crate) fn clamp_player(&mut self) {
        self.signal(Signal::ClampPlayer);
    }
    pub(crate) fn game_flag(&mut self, mask: u16) {
        self.signal(Signal::GameFlags(mask));
    }
    pub(crate) fn fade_white(&mut self, line: u8) {
        self.palette_op(PaletteOp::FadeWhite(line));
    }
    pub(crate) fn fade_palette(&mut self, line: u8, target: [u16; 16]) {
        self.palette_op(PaletteOp::FadeTo(line, target));
    }
    pub(crate) fn fill_palette(&mut self, line: u8, color: u16) {
        self.palette_op(PaletteOp::Fill(line, color));
    }
    pub(crate) fn restore_palette(&mut self, line: u8) {
        self.palette_op(PaletteOp::Restore(line));
    }
    pub(crate) fn palette_op(&mut self, op: PaletteOp) {
        self.signal(Signal::Palette(op));
    }
    pub(crate) fn plane_op(&mut self, op: PlaneOp) {
        self.signal(Signal::Plane(op));
    }
    /// `$96EE`: r = r * 5 + 1.
    pub(crate) fn random(&mut self) -> u16 {
        next_random(self.random)
    }
    pub(crate) fn play(&mut self, command: u8) {
        self.sounds.push(command);
    }
    /// `$4A4E`: request effect `id` of the `$5B56` table.
    pub(crate) fn effect_sound(&mut self, id: u8) {
        if let Some(command) = self.shared.effects.command(id) {
            self.sounds.push(command);
        }
    }
    /// `$4B2E`: effect `id`, only for an object inside `$4B52`'s window.
    pub(crate) fn visible_sound(&mut self, x: i32, y: i32, id: u8) {
        if crate::combat::in_sound_window((x, y), self.camera) {
            self.effect_sound(id);
        }
    }
    /// Native removal bound for effects and the forest set.
    pub(crate) fn offscreen(&self, x: i32, y: i32) -> bool {
        x < self.camera.0 + self.view.0 - 96
            || x > self.camera.0 + self.view.1 + 96
            || y > self.camera.1 + 320
    }
    /// Object visibility against this pass's expanded screen.
    pub(crate) fn outside_view(&self, x: i32, y: i32) -> bool {
        crate::combat::outside_view_in((x, y), self.camera, self.view)
    }
    /// `obj_dx` bit 15 (`$95D6`): the player is left of `x`.
    pub(crate) fn player_left(&self, x: i32) -> bool {
        self.player.0 < x
    }
    /// `obj_dx & $7FFF` (`$38`): horizontal distance to the player.
    pub(crate) fn player_dx(&self, x: i32) -> i32 {
        (self.player.0 - x).abs()
    }
    /// `obj_dy & $7FFF` (`$3A`): vertical distance to the player.
    pub(crate) fn player_dy(&self, y: i32) -> i32 {
        (self.player.1 - y).abs()
    }
    /// Graphics base of type `kind`: the first object art block starts at
    /// `$2300`, which also stands in for a type whose art is not loaded.
    /// Art only; [`Ctx::tile_base`] is the `obj_tile` a handler sets.
    pub(crate) fn base(&self, kind: u16) -> u16 {
        self.bases.get(&kind).copied().unwrap_or(0x2300)
    }
    /// `Object_SetTileBase` (`$12B6A`) with d1 = `$2300` (or `$A300`, the
    /// same once masked): type `kind`'s graphics base with bit 15 lifted
    /// from the current object's `obj_spawn_flags` bit 14.
    pub(crate) fn tile_base(&self, kind: u16) -> u16 {
        (self.base(kind) & 0x7fff) | super::spawn_priority(self.spawn_flags)
    }
    /// `Func_012BFE`: a player attack object within `reach` pixels of
    /// `x + dx` on the player's side, strictly above `y` by at most `rise`.
    pub(crate) fn attack_near(&self, x: i32, y: i32, dx: i32, reach: i32, rise: i32) -> bool {
        let left = self.player_left(x);
        let priority = self.current_high_priority();
        self.attacks.iter().any(|&(ax, ay, high_priority)| {
            let d = ax - (x + dx);
            high_priority == priority
                && (d < 0) == left
                && d.abs() <= reach
                && ay < y
                && y - ay <= rise
        })
    }
    pub(crate) fn center(&self, enemy: &Enemy) -> (i32, i32) {
        if enemy.body.is_none() && !self.mappings.contains_key(&enemy.mapping) {
            return (enemy.x, enemy.y - 16);
        }
        let hitbox = crate::combat::enemy_hitbox(self.mappings, enemy);
        (hitbox.x, hitbox.y)
    }
    pub(crate) fn effect(&mut self, effect: Effect) {
        self.effects.push(effect);
    }
    /// `Func_012E14`: pose and the shared death arc (vy -3.5, gravity $3000,
    /// 1 px/tick away from the player). The caller advances `routine`.
    pub(crate) fn death_arc(&self, enemy: &mut Enemy, mapping: u16) {
        let fall = self.shared.fall;
        enemy.mapping = mapping;
        enemy.motion.vy = fall.vy;
        enemy.motion.gravity = fall.gravity;
        enemy.motion.vx = if self.player_left(enemy.x) {
            fall.vx
        } else {
            -fall.vx
        };
    }
    /// `Func_012E3E`: the ordinary explosion (`$36DAE`) once the object is
    /// off screen, or falling (vy and gravity words of one sign) onto
    /// terrain. True when it exploded; the caller frees the object.
    pub(crate) fn landed_blast(&mut self, enemy: &Enemy, terrain: &dyn Terrain) -> bool {
        let falling = ((enemy.motion.vy >> 16) < 0) == (enemy.motion.gravity < 0);
        if enemy.offscreen || (falling && terrain.solid_at(enemy.x, enemy.y)) {
            let blast = self.shared.blasts[0];
            self.explode(enemy.x, enemy.y, blast);
            return true;
        }
        false
    }
    /// A list-1 shell casing (`casings[kind]`) at (x, y), ejected away from
    /// the facing of a parent whose flip is `facing_left`.
    pub(crate) fn casing(&mut self, x: i32, y: i32, facing_left: bool, kind: usize) {
        let casing = self.shared.casings[kind];
        let mut piece = Effect::new(EffectKind::Pieces, x, y, casing.mapping);
        piece.motion = Motion {
            vx: if facing_left { casing.vx } else { -casing.vx },
            vy: casing.vy,
            gravity: casing.gravity,
            ..Motion::default()
        };
        piece.counter = casing.life;
        self.effect(piece);
    }
    /// `Object_AllocL4` for a ROM-style type: false when list 4 is full.
    pub(crate) fn shoot(&mut self, projectile: Projectile) -> bool {
        self.shoot_capped(projectile, LIST4_CAPACITY)
    }
    /// `Object_AllocL4` for a shot whose allocator writes its own `obj_tile`
    /// word rather than copying the thrower's (`Func_012DD8`): the shot's
    /// priority is bit 15 of `tile` (its art stays `Projectile::base`).
    pub(crate) fn shoot_tiled(&mut self, projectile: Projectile, tile: u16) -> bool {
        self.shoot_tiled_capped(projectile, tile, LIST4_CAPACITY)
    }
    /// A boss can raise `Object_ListParams+2` before allocating tiled shots.
    pub(crate) fn shoot_tiled_capped(
        &mut self,
        mut projectile: Projectile,
        tile: u16,
        cap: usize,
    ) -> bool {
        if !list4_has_space(self.projectiles, cap) {
            return false;
        }
        projectile.high_priority = super::tile_priority(tile);
        self.allocate_projectile(projectile);
        true
    }
    /// `Object_AllocL4` under an `Object_ListParams+2` limit of `cap` enemy
    /// records (bosses raise the limit for their own shots).
    pub(crate) fn shoot_capped(&mut self, projectile: Projectile, cap: usize) -> bool {
        if !list4_has_space(self.projectiles, cap) {
            return false;
        }
        let mut projectile = projectile;
        projectile.high_priority = self.current_high_priority();
        self.allocate_projectile(projectile);
        true
    }
    fn allocate_projectile(&mut self, mut projectile: Projectile) {
        // Object_InitRecord and the ninja allocator do not clear +$28.
        // Other native shots keep their explicit animation initialization.
        let counter = self.projectile_scratch.pop().unwrap_or(0);
        if (0x35b..=0x35e).contains(&projectile.mapping) {
            projectile.animation_counter = counter;
        }
        projectile.scratch_slot = true;
        self.projectiles.push(projectile);
    }
    /// A non-list-4 collision object owned by the current handler. Boss
    /// composite hitboxes use this path and still inherit `obj_tile` priority.
    pub(crate) fn push_projectile_unchecked(&mut self, mut projectile: Projectile) {
        projectile.high_priority = self.current_high_priority();
        self.projectiles.push(projectile);
    }
    /// `Object_AllocL5`: a child of the current object with `routine` 0
    /// unless the caller presets it, linked at the tail of the list: its
    /// first update comes later in this pass (`Combat::update_enemies`).
    /// `None` when list 5 is full or `kind` is not a type offset.
    pub(crate) fn spawn(&mut self, kind: u16, x: i32, y: i32) -> Option<&mut Enemy> {
        if self.live + self.children.len() >= LIST5_CAPACITY || !kind.is_multiple_of(4) {
            return None;
        }
        let shared = self.shared;
        let health = shared.stats.get(usize::from(kind / 4))?.health_counter;
        let mut enemy = Enemy::new(usize::MAX, x, y, 0, u16::from(health) + 1);
        enemy.state = EnemyState::Waiting;
        enemy.parent = Some(self.current);
        enemy.high_priority = self.current_high_priority();
        if kind == 0xfc {
            // Func_01B8A8, the only $FC allocator, counts it in $FF3D05.
            self.mutants.released();
        }
        self.children.push(Child { kind, enemy });
        self.children.last_mut().map(|child| &mut child.enemy)
    }
    /// The `$3C` parent of `enemy`, as it was before this pass (after its
    /// update, for a record allocated during the pass).
    pub(crate) fn parent(&self, enemy: &Enemy) -> Option<Snapshot> {
        enemy
            .parent
            .and_then(|index| self.others.get(index).copied())
    }
    /// `Func_0129B8` (`Object_FreeL5`): removed without a death sequence.
    pub(crate) fn free(enemy: &mut Enemy) {
        enemy.state = EnemyState::Defeated;
        enemy.dying = false;
        enemy.mapping = 0;
        enemy.motion.stop();
    }
    /// End of a death sequence: the object is removed.
    pub(crate) fn remove(enemy: &mut Enemy) {
        enemy.dying = false;
        enemy.motion.stop();
    }
    /// `Object_ProcessHit` (`$127C0`) as seen by a ROM-style handler: applies
    /// the hit recorded by `Combat::hurt_enemy` at the top of the update.
    /// Lethal: `obj_saved_routine` = routine, routine = `death_routine`.
    /// Otherwise routine = `hurt_routine`. Sparks as `$36900`/`$368FA`.
    pub(crate) fn process_hit(&mut self, enemy: &mut Enemy) -> Option<Reaction> {
        let reaction = enemy.reaction.take();
        match reaction {
            Some(Reaction::Death) => {
                if enemy.death_spark {
                    self.spark(enemy, true);
                }
                enemy.saved_routine = enemy.routine;
                enemy.routine = enemy.death_routine;
            }
            Some(Reaction::Hurt) => {
                self.spark(enemy, false);
                enemy.saved_routine = enemy.routine;
                enemy.routine = enemy.hurt_routine;
                enemy.state = EnemyState::Waiting;
            }
            Some(Reaction::Deflect) => {
                let sound = self.shared.deflect_sound;
                self.play(sound);
                self.spark(enemy, false);
            }
            None => {}
        }
        reaction
    }
    /// `Func_0128B2`: a recorded hit is applied and the routine body skipped.
    pub(crate) fn hit_taken(&mut self, enemy: &mut Enemy) -> bool {
        self.process_hit(enemy)
            .is_some_and(|reaction| reaction != Reaction::Deflect)
    }
    /// `$1518C`: a leaf within 31 pixels of the object.
    pub(crate) fn leaf(&mut self, x: i32, y: i32) {
        let dx = i32::from(self.random() & 63) - 31;
        let dy = i32::from(self.random() & 63) - 31;
        let mut leaf = Effect::new(EffectKind::Leaf, x + dx, y + dy, self.shared.leaf_mapping);
        leaf.base = self.shared.leaf_base;
        leaf.counter = 0;
        self.effect(leaf);
    }
    /// `$36DB6` + `$36E82`: ten puffs, one per tick, rising from the feet.
    pub(crate) fn explode(&mut self, x: i32, y: i32, blast: Blast) {
        explode(
            self.shared,
            self.random,
            self.effects,
            self.sounds,
            (x, y),
            blast,
            self.current_high_priority(),
        );
    }
    /// `Func_013CE0`: four fire puffs around a body at (x, y).
    pub(crate) fn fire_puffs(&mut self, x: i32, y: i32) {
        for _ in 0..4 {
            let dy = -i32::from(self.random() & 0x3f);
            let dx = i32::from(self.random() & 0x3f) - 0x20;
            let angle = self.random() & 0x3f0;
            let (cos, sin) = self.shared.sine.polar(angle);
            let mut puff = Effect::new(EffectKind::Puff, x + dx, y + dy, 0);
            puff.motion = Motion {
                vx: cos << 1,
                vy: sin << 1,
                ..Motion::default()
            };
            self.effect(puff);
        }
    }
    /// `$36EEE`: four debris, three sparkles and the debris sound.
    pub(crate) fn burst(&mut self, x: i32, y: i32) {
        let shared = self.shared;
        for angle in shared.debris_angles {
            let (cos, sin) = shared.sine.polar(angle);
            let mut piece = Effect::new(EffectKind::Debris, x, y, shared.debris_seq[0]);
            piece.motion = Motion {
                vx: cos << 1,
                vy: sin << 2,
                gravity: shared.debris_gravity,
                ..Motion::default()
            };
            self.effect(piece);
        }
        for angle in shared.sparkle_angles {
            let (cos, sin) = shared.sine.polar(angle);
            let mut piece = Effect::new(EffectKind::Sparkle, x, y, shared.sparkle_seq[0]);
            // $36A38 folds this into a quirky orbit; native keeps its angles/speeds.
            piece.motion = Motion {
                vx: cos << 1,
                vy: sin << 1,
                gravity: shared.sparkle_gravity,
                ..Motion::default()
            };
            self.effect(piece);
        }
        self.sounds.push(shared.debris_sound);
    }
    /// `$36EEE` + `$15FE8`: the burst and the type-$24 flying head.
    pub(crate) fn debris(&mut self, enemy: &Enemy, base: u16) {
        self.burst(enemy.x, enemy.y);
        let [mapping, dx, dy, vy, gravity, bounces] = self.shared.head;
        let mut head = Effect::new(EffectKind::Head, enemy.x + dx, enemy.y + dy, mapping as u16);
        head.base = base;
        head.flip = enemy.facing_left;
        head.counter = bounces;
        head.motion = Motion {
            vx: enemy.motion.vx >> 1,
            vy,
            gravity,
            ..Motion::default()
        };
        self.effect(head);
    }
    /// `$3671C`: eight radial sparkles.
    pub(crate) fn poof(&mut self, x: i32, y: i32) {
        let shared = self.shared;
        for angle in shared.poof_angles {
            let (cos, sin) = shared.sine.polar(angle);
            let mut piece = Effect::new(EffectKind::Poof, x, y, shared.poof_seq[0]);
            piece.motion = Motion {
                vx: (cos >> 1) + (cos >> 2),
                vy: (sin >> 1) + (sin >> 2),
                ..Motion::default()
            };
            self.effect(piece);
        }
    }
    /// `Func_031A7A`: allocate a list-1 `$1B` sparkle 16 px away from
    /// `at`, in one of the five `Data_031AD6` angle ranges, moving back to
    /// `at` at one pixel per tick. `angle_set` is the ROM's D1 / 4.
    pub(crate) fn charge_sparkle(&mut self, at: (i32, i32), angle_set: usize) {
        let shared = self.shared;
        let Some(&(mask, add)) = shared.charge_sparkle_angles.get(angle_set) else {
            return;
        };
        let angle = (self.random() & mask).wrapping_add(add);
        let (cos, sin) = shared.sine.polar(angle);
        let (mut x, mut y) = at;
        let mut motion = Motion {
            vx: -cos,
            vy: -sin,
            ..Motion::default()
        };
        crate::combat::advance(&mut x, &mut motion.fx, cos << 4);
        crate::combat::advance(&mut y, &mut motion.fy, sin << 4);
        if let Some(&(_, mapping)) = shared.charge_sparkle.first() {
            let mut sparkle = Effect::new(EffectKind::ChargeSparkle, x, y, mapping);
            sparkle.base = shared.charge_sparkle_base;
            sparkle.high_priority = super::tile_priority(sparkle.base);
            sparkle.motion = motion;
            self.effect(sparkle);
        }
    }
    /// `Func_036FF2` (see [`SmallBlast`](super::shared::SmallBlast)).
    pub(crate) fn small_blast(&mut self, x: i32, y: i32) {
        let first = self.effects.len();
        let high = self.current_high_priority();
        small_blast(
            self.shared,
            self.random,
            self.effects,
            self.sounds,
            self.camera,
            (x, y),
        );
        // $37044 -> $12DD8: the burst chain inherits its parent's bit 15.
        for puff in &mut self.effects[first..] {
            puff.base = if high { 0x8000 } else { 0 };
            puff.high_priority = high;
        }
    }
    /// Request a `Layout_Buffer` metatile write (`Func_015054`, or
    /// `Func_015074` with its `Func_0150BA` name-table cells): the game
    /// applies it after this pass, changing collision and plane A art.
    /// Used by the 3-1 collapse (`Func_0197FE`) and the 3-1 tanks'
    /// `Data_019014` broken glass (`Func_01903A`).
    pub(crate) fn edit_terrain(&mut self, x: i32, y: i32, metatile: u8) {
        self.signal(Signal::Terrain(crate::stage::TerrainEdit {
            x,
            y,
            metatile,
        }));
    }
    /// `Func_015054`: every (metatile, dx, dy) of a [`metatile_list`]
    /// around (`x`, `y`), such as the 3-1 tanks' broken glass.
    ///
    /// [`metatile_list`]: super::metatile_list
    pub(crate) fn write_metatiles(&mut self, x: i32, y: i32, list: &[(u8, i32, i32)]) {
        for &(metatile, dx, dy) in list {
            self.edit_terrain(x + dx, y + dy, metatile);
        }
    }
    /// A one-tick sprite drawn this frame only, in list 1.
    pub(crate) fn overlay(&mut self, x: i32, y: i32, mapping: u16, base: u16, flip: bool) {
        self.overlay_in(SpriteList::Effects, x, y, mapping, base, flip);
    }
    /// A one-tick sprite drawn this frame only for a part that is a record
    /// of sprite list `list` in the cartridge and follows its parent, such
    /// as the `$DC` barrel or the `$60`/`$70` wake (list 9).
    pub(crate) fn overlay_in(
        &mut self,
        list: SpriteList,
        x: i32,
        y: i32,
        mapping: u16,
        base: u16,
        flip: bool,
    ) {
        let mut piece = Effect::new(EffectKind::Pieces, x, y, mapping);
        piece.base = base;
        piece.flip = flip;
        piece.back = list == SpriteList::Back;
        piece.list = if piece.back {
            SpriteList::Effects
        } else {
            list
        };
        // Kept by this frame's effect pass, freed by the next one's.
        piece.counter = 1;
        self.effect(piece);
    }
    /// `Player_SpawnDust` (`$228EC`) called by an object at (x, y).
    pub(crate) fn dust(&mut self, x: i32, y: i32) {
        let mut dust = Effect::new(EffectKind::Dust, x, y, self.shared.dust_seq[0]);
        dust.motion.vy = -0x10000;
        dust.blink = true;
        self.effect(dust);
    }
    /// `$368FA` (non-lethal) / `$36900` (lethal) hit spark at the body.
    pub(crate) fn spark(&mut self, enemy: &Enemy, lethal: bool) {
        if enemy.spark_handled {
            return;
        }
        let (x, y) = self.center(enemy);
        let (kind, first) = if lethal {
            (EffectKind::KillSpark, self.shared.kill_seq[0])
        } else {
            (EffectKind::HitSpark, self.shared.hit_seq[0])
        };
        self.effect(Effect::new(kind, x, y, first));
    }
    pub(crate) fn update_effects(&mut self, terrain: &dyn Terrain) {
        let shared = self.shared;
        let camera = self.camera;
        let props = &shared.leader_props;
        let mut chips = Vec::new();
        for effect in self.effects.iter_mut() {
            // Its boss runs it (and frees it) itself.
            if matches!(effect.kind, EffectKind::BossPart | EffectKind::Boss7Pillar) {
                continue;
            }
            if matches!(
                effect.kind,
                EffectKind::GuardPart
                    | EffectKind::ThrownRider
                    | EffectKind::HorseWreck
                    | EffectKind::SurfSlashBoard
            ) {
                // Driven by their owners (`enemies::round4::escort`,
                // `special::thrown`).
                continue;
            }
            // Allocated on a later frame: hidden until the pass that
            // counts its delay out gives it its first update.
            if effect.delay > 0 {
                effect.delay -= 1;
                if effect.delay > 0 {
                    continue;
                }
            }
            effect.age = effect.age.saturating_add(1);
            if crate::combat::ambient::owns(effect.kind) {
                crate::combat::ambient::update(shared, effect, camera, terrain);
                continue;
            }
            let sequence = |seq: &Vec<u16>, period: u16, age: u16| {
                seq.get(usize::from((age - 1) / period)).copied()
            };
            let frame = match effect.kind {
                EffectKind::KaruraSpark | EffectKind::KaruraFeather | EffectKind::KaruraTrail => {
                    shared.karura.update(effect, terrain, self.random)
                }
                EffectKind::MechParticle => {
                    let counter = effect.age - 1 + effect.counter as u16;
                    if effect.phase == 0 {
                        shared.mech_smoke.iter().find(|(last, _, _)| counter <= *last)
                            .map(|(_, mapping, attr)| {
                                effect.base = 0x8000 | attr;
                                *mapping
                            })
                    } else {
                        frame_at(&shared.charge_sparkle, counter).map(|(_, mapping, _)| mapping)
                    }
                }
                EffectKind::Dust | EffectKind::WallSpark => {
                    sequence(&shared.dust_seq, 1, effect.age)
                }
                EffectKind::Puff => sequence(
                    &shared.puff_seq,
                    if effect.period == 0 { 2 } else { effect.period },
                    effect.age,
                ),
                EffectKind::Debris => sequence(&shared.debris_seq, 2, effect.age),
                EffectKind::Sparkle => {
                    effect.flip = !effect.flip;
                    sequence(&shared.sparkle_seq, 3, effect.age)
                }
                EffectKind::HitSpark => sequence(&shared.hit_seq, 1, effect.age),
                EffectKind::KillSpark => sequence(&shared.kill_seq, 1, effect.age),
                EffectKind::DeflectSpark => sequence(&shared.deflect_seq, 1, effect.age),
                EffectKind::Poof => sequence(&shared.poof_seq, 8, effect.age),
                EffectKind::ChargeSparkle => {
                    frame_at(&shared.charge_sparkle, effect.age).map(|(_, mapping, _)| mapping)
                }
                EffectKind::Leaf => {
                    if effect.phase == 0 {
                        // $151FA: eight eight-tick wiggles from the $151E2 bytes.
                        if effect.age.is_multiple_of(8) {
                            effect.x += shared.leaf_wiggle[3 - (effect.counter as usize & 3)];
                            effect.counter += 1;
                            if effect.counter == 8 {
                                effect.phase = 1;
                                effect.angle = 3;
                                effect.counter = i32::from(shared.leaf_sway) + 1;
                                effect.motion.gravity = shared.leaf_gravity;
                            }
                        }
                    } else {
                        // $15250: sway with twice the cosine, facing the drift.
                        effect.angle = effect.angle.wrapping_add(16);
                        effect.motion.vx = shared.sine.polar(effect.angle).0 << 1;
                        effect.flip = effect.motion.vx < 0;
                        effect.counter -= 1;
                        if effect.counter < 0 {
                            effect.alive = false;
                        }
                    }
                    Some(effect.mapping)
                }
                EffectKind::Head => {
                    // $1609A consumes last frame's visibility only after
                    // its last bounce; $16044 itself does not cull.
                    if effect.phase != 0 && effect.flags & 0x80 != 0 {
                        effect.alive = false;
                    }
                    // $16044: flip cycle every eight ticks, bounce at 3/4 speed.
                    let flips = shared.head_flips[usize::from((effect.age >> 3) & 3)];
                    effect.base = (effect.base & !0x1800) | flips;
                    if effect.phase == 0 && terrain.solid_at(effect.x, effect.y) {
                        effect.counter -= 1;
                        if effect.counter < 0 {
                            effect.phase = 1;
                        } else {
                            effect.y &= !15;
                            let up = -effect.motion.vy >> 1;
                            effect.motion.vy = up + (up >> 1);
                        }
                    }
                    Some(effect.mapping)
                }
                EffectKind::Flash => {
                    frame_at(&shared.muzzle, effect.age - 1).map(|(_, mapping, _)| mapping)
                }
                // Updated by `ambient::update` above, or by their boss.
                EffectKind::Boss4Launcher
                | EffectKind::BossPart
                | EffectKind::Boss7Pillar
                | EffectKind::AmbientLeaf
                | EffectKind::Drip
                | EffectKind::Pebble
                | EffectKind::Splash
                | EffectKind::Wake
                | EffectKind::Plume
                // Skipped above.
                | EffectKind::GuardPart
                | EffectKind::ThrownRider
                | EffectKind::HorseWreck
                | EffectKind::SurfSlashBoard => Some(effect.mapping),
                EffectKind::Ghost => {
                    effect.base ^= 0x4000;
                    effect.counter -= 1;
                    if effect.counter < 0 {
                        effect.alive = false;
                    }
                    Some(effect.mapping)
                }
                EffectKind::Boulder => props
                    .boulder(effect, terrain, self.random, &mut chips)
                    .then_some(effect.mapping),
                EffectKind::Chip => props.chip_step(effect).then_some(effect.mapping),
                EffectKind::Gate => {
                    props.gate(effect, self.mappings, self.player, &mut self.signals);
                    Some(effect.mapping)
                }
                EffectKind::Orbit | EffectKind::Boss4Charge => {
                    // The previous update's `addq`/`subq`s (none before the
                    // first, `$262B6`), then this one's place.
                    if effect.age > 1 {
                        effect.angle = effect.angle.wrapping_add(u16::from(effect.phase));
                        effect.counter -= 1;
                        if (effect.counter + 1) & 7 == 0 {
                            effect.mapping -= 1;
                        }
                    }
                    place_orbit(&shared.sine, effect);
                    (effect.counter > 0).then_some(effect.mapping)
                }
                EffectKind::Flare => {
                    if effect.counter & 1 == 0 {
                        effect.angle = (effect.angle + 1) & 3;
                        effect.flip = effect.angle & 1 != 0;
                        effect.base = (effect.base & !0x1000)
                            | if effect.angle & 2 != 0 { 0x1000 } else { 0 };
                    }
                    effect.counter -= 1;
                    if effect.counter <= 0 {
                        effect.counter = i32::from(effect.period.max(1));
                        effect.phase = effect.phase.saturating_sub(1);
                        effect.mapping += 1;
                    }
                    (effect.phase > 0).then_some(effect.mapping)
                }
                EffectKind::Petal => {
                    let (cos, _) = shared.sine.polar(effect.angle.wrapping_mul(16));
                    effect.x = effect.origin.0 + ((cos * 40) >> 16);
                    effect.angle = (effect.angle + 1) & 127;
                    (effect.y <= camera.1 + 200).then_some(effect.mapping)
                }
                EffectKind::Pieces => {
                    // `phase` counts the landings left; 0 passes through terrain.
                    if effect.phase > 0
                        && effect.motion.vy >= 0
                        && terrain.solid_at(effect.x, effect.y)
                    {
                        effect.phase -= 1;
                        if effect.phase == 0 {
                            effect.alive = false;
                        } else {
                            effect.y &= !15;
                            effect.motion.vy = -(effect.motion.vy >> 1);
                        }
                    }
                    effect.counter -= 1;
                    if effect.counter < 0 {
                        effect.alive = false;
                    }
                    Some(effect.mapping)
                }
            };
            match frame {
                Some(mapping) => effect.mapping = mapping,
                None => effect.alive = false,
            }
            effect.motion.step(&mut effect.x, &mut effect.y);
            // $17A4 only marks visibility. Sequence/countdown handlers
            // retain their records until their own terminator, even off screen.
            effect.flags = (effect.flags & !0x80)
                | if crate::combat::ambient::off_screen((effect.x, effect.y), camera) {
                    0x80
                } else {
                    0
                };
        }
        self.effects.retain(|effect| effect.alive);
        for chip in chips {
            self.effect(chip);
        }
    }
}

/// `$262D6`: an [`EffectKind::Orbit`] at radius `counter` + 8 from its
/// origin (`$94A0`, `asr.l #2`, `muls.w`, `lsl.l #2`, `swap`).
pub(crate) fn place_orbit(sine: &crate::rom::Sine, effect: &mut Effect) {
    let radius = effect.counter + 8;
    let (dx, dy) = crate::ninjutsu_fx::orbit(sine, i32::from(effect.angle), radius);
    effect.x = effect.origin.0 + dx;
    effect.y = effect.origin.1 + dy;
}

/// `Func_0096EE`: advance the shared generator `$FF001C` (r * 5 + 1).
pub(crate) fn next_random(state: &mut u16) -> u16 {
    *state = state.wrapping_mul(5).wrapping_add(1);
    *state
}

/// `Func_009562`: a 16.16 direction from `from` to `to` whose dominant axis
/// is 1.0 and whose minor axis is the ratio (`divu`); equal distances give
/// (1, 1) and a zero axis gives the pure other axis.
pub(crate) fn aim(from: (i32, i32), to: (i32, i32)) -> (i32, i32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let (ax, ay) = (dx.abs(), dy.abs());
    let (x, y) = if ax == 0 {
        (0, 0x10000)
    } else if ay == 0 {
        (0x10000, 0)
    } else if ax == ay {
        (0x10000, 0x10000)
    } else if ax > ay {
        (0x10000, ((ay << 16) / ax).min(0xffff))
    } else {
        (((ax << 16) / ay).min(0xffff), 0x10000)
    };
    (if dx < 0 { -x } else { x }, if dy < 0 { -y } else { y })
}
