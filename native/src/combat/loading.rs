use super::*;

impl Combat {
    /// Spawns, object graphics bases and resident art of any part.
    pub fn load_stage(rom: &[u8], stage: crate::stage::StageId) -> Result<Self> {
        let mut spawns = stage_spawns(rom, stage)?;
        // Every loadable `$43222` record (959 ids): player, projectiles,
        // effects and all enemy poses, so no handler can select a missing one.
        let mut mappings = BTreeMap::new();
        for id in 0..crate::rom::SPRITE_MAPPING_COUNT {
            if let Ok(mapping) = Mapping::load(rom, id) {
                mappings.insert(id, mapping);
            }
        }
        for id in [1, 0xca, 0xd2, 0xd3, 0xd5, 0xd6, 0xd7, 0xd8, 0x35b] {
            if !mappings.contains_key(&id) {
                return Err(format!("combat mapping ${id:x} does not load"));
            }
        }
        let player_mappings = (0..PLAYER_FRAME_COUNT)
            .map(|frame| word(rom, PLAYER_FRAME_TABLE + frame * 4))
            .collect::<Result<Vec<_>>>()?;
        if let Some(id) = player_mappings.iter().find(|id| !mappings.contains_key(id)) {
            return Err(format!("player mapping ${id:x} does not load"));
        }
        let rules = Shared::load(rom)?;
        let roster = Roster::load(rom, stage)?;
        let prefix = object_selection(rom, stage)?;
        let object_root = pointer(rom, 0x80008)?;
        let indices = pointer(rom, object_root)?;
        let groups = pointer(rom, object_root + 4)?;
        let mut bases = BTreeMap::new();
        let mut base: u16 = 0;
        for i in 0..4 {
            let p = prefix + i * 8;
            if word(rom, p)? == 0xffff {
                break;
            }
            let kind = word(rom, p + 6)?;
            bases.insert(kind, 0x2300u16.wrapping_add(base));
            let mut group = pointer(
                rom,
                groups + crate::rom::raw_pointer(rom, indices + usize::from(kind))?,
            )?;
            let mut terminated = false;
            for _ in 0..256 {
                let destination = word(rom, group)?;
                let descriptor = word(rom, group + 2)?;
                group += 4;
                if destination == 0xffff {
                    base = base
                        .checked_add(descriptor)
                        .ok_or("object graphics base overflow")?;
                    terminated = true;
                    break;
                }
            }
            if !terminated {
                return Err("unterminated object graphics group".into());
            }
        }
        // $1C628 deliberately shares type-$B4 graphics for the $B8 entry.
        if let Some(&base) = bases.get(&0xb4) {
            bases.insert(0xb8, base);
        }
        let mut common_tiles = BTreeMap::new();
        let mut puff_tiles = BTreeMap::new();
        // Resident art: all eight $8001C blocks ($4D8..$575: projectiles,
        // sparks and explosion puffs), Joe, the HUD, and the hit sparks
        // $3C422/$3C4B8/$3C4EC ($5B0..$5FF) loaded by $A2EA..$A308.
        let common = pointer(rom, 0x8001c)?;
        let puffs = pointer(rom, common + 0x1c)?;
        let mut sources = Vec::new();
        for i in 0..8 {
            sources.push(pointer(rom, common + i * 4)?);
        }
        sources.extend([
            pointer(rom, pointer(rom, 0x80000)?)?,
            0x3bffa,
            0x3c422,
            0x3c4b8,
            0x3c4ec,
        ]);
        let mode = spawns::mode_flags(rom, stage)?;
        // $A32E: over the font's last cells, mounted stages load the warning
        // sign ($3C646, tiles $30..$38, `crate::special::Warnings`), the
        // others the "GO" markers ($3C570, tiles $30..$3C).
        sources.push(if mode.mounted() { 0x3c646 } else { 0x3c570 });
        // $A30E: arenas (`$FF39F7` & 3) skip the puff block, whose tiles
        // $4D8..$4F9 hold boss or miniboss art there instead.
        let resident_puffs = mode.resident_puffs();
        for source in sources {
            let first = usize::from(word(rom, source)?);
            let count = usize::from(word(rom, source + 2)?) + 1;
            if first + count > 2048 {
                return Err("combat resident art exceeds VRAM".into());
            }
            let (data, _) = musashi(rom, source + 4, 65536)?;
            let bytes = data
                .get(..count * 32)
                .ok_or("truncated combat resident art")?;
            for (i, tile) in bytes.as_chunks::<32>().0.iter().enumerate() {
                if source == puffs {
                    puff_tiles.insert(first + i, Tile(*tile));
                    if !resident_puffs {
                        continue;
                    }
                }
                common_tiles.insert(first + i, Tile(*tile));
            }
        }
        let mut powered_tiles = BTreeMap::new();
        {
            let source = pointer(rom, pointer(rom, 0x80000)? + 4)?;
            let first = usize::from(word(rom, source)?);
            let count = usize::from(word(rom, source + 2)?) + 1;
            let (data, _) = musashi(rom, source + 4, 65536)?;
            let bytes = data
                .get(..count * 32)
                .ok_or("truncated powered resident art")?;
            for (i, tile) in bytes.as_chunks::<32>().0.iter().enumerate() {
                powered_tiles.insert(first + i, Tile(*tile));
            }
        }
        let mut enemies: Vec<Enemy> = spawns
            .iter()
            .enumerate()
            .map(|(index, s)| {
                Enemy::new(
                    index,
                    s.x,
                    s.y,
                    roster.initial_mapping(s),
                    u16::from(s.health_counter) + 1,
                )
            })
            .collect();
        // Boss_Setup ($2A3AE) at stage load: Boss_Spawn allocates the list-8
        // record from Boss_Records[Round_Index] in boss arenas. It runs every
        // frame from the start (Boss_Update), not through spawn activation.
        if let Some(record) = crate::bosses::record(rom, stage)? {
            let (spawn, enemy) = crate::bosses::spawn(&record, spawns.len());
            spawns.push(spawn);
            enemies.push(enemy);
        }
        let max_health = *rom.get(0x615f).ok_or("missing default health")?;
        Ok(Self {
            weapon_contacts: Vec::new(),
            spawns,
            enemies,
            projectiles: Vec::new(),
            effects: Vec::new(),
            rules,
            roster,
            random: 0,
            mappings,
            player_mappings,
            player_frame: 1,
            player_input: crate::input::Input::default(),
            bases,
            common_tiles,
            puff_tiles,
            powered_tiles,
            hud: Hud {
                health: max_health,
                max_health,
                lives: rom[0x615e],
                ammo: u16::from(rom[0x6174]),
                score: 0,
                score_display: 0,
                // Native session timer: the original HUD does not show a stage clock.
                timer_seconds: 0,
            },
            initial_ammo: u16::from(rom[0x6174]),
            view_width: crate::WIDTH,
            world_bounds: (0, i32::MAX),
            view_screen_clamped: false,
            invulnerability: 0,
            invincible: false,
            player_priority_raised: crate::stage::sections(
                rom,
                stage.round_index(),
                stage.substage_index(),
            )?
            .get(stage.part_index())
            .is_some_and(|section| crate::stage::ModeFlags(section.raw[7]).player_high_priority()),
            ticks: 0,
            camera_locked: false,
            mech_hand_contact: false,
            allocations: 0,
            projectile_scratch: Vec::new(),
            game_over: false,
            dying: false,
            attack_serial: 0,
            enemy_projectile_damage: *rom.get(0x1fbfa).ok_or("missing projectile damage")?,
            sound_commands: [
                SoundEvent::Throw.command(rom)?,
                SoundEvent::EnemyStar.command(rom)?,
                SoundEvent::PlayerHurt.command(rom)?,
            ],
            weapons: PlayerWeapons::load(rom)?,
            shield_art: crate::ninjutsu_fx::Shield::load(rom)?,
            pending_damage: 0,
            landed_shot: None,
            struck_owner: None,
            pending_player_event: None,
            art_reloaded: false,
            ambient: ambient::Ambient::new(
                crate::stage::sections(rom, stage.round_index(), stage.substage_index())?
                    .get(stage.part_index())
                    .map_or(0, |section| section.raw[14]),
            ),
            pending_hit: None,
            mount: None,
            checked: false,
            carried: Carried::default(),
            player_hidden: false,
            object_contact: None,
            shove: None,
            deferring: false,
            holds: 0,
            score_suppressed: false,
            level: 0,
            item_boxes: Vec::new(),
            mutants: enemies::round3::Mutants::default(),
        })
    }
}
