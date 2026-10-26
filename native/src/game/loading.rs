use super::*;
use crate::content::{Cartridge, StageSource};
use std::sync::Arc;

impl Game {
    /// Round 1-1, part 0.
    pub fn load(rom: &[u8]) -> Result<Self, String> {
        Self::load_stage(rom, StageId::FOREST)
    }
    /// Any original round/substage, starting in `id.part`.
    pub fn load_stage(rom: &[u8], id: StageId) -> Result<Self, String> {
        Self::load_from(rom, Cartridge::source(), id)
    }
    /// Substage `id` of `source`'s content, starting in `id.part`. Every
    /// part of the substage is loaded up front so stream handoffs never
    /// stall.
    pub fn load_from(
        rom: &[u8],
        source: Arc<dyn StageSource>,
        id: StageId,
    ) -> Result<Self, String> {
        let sections = source.sections(rom, id)?;
        let part = id.part_index();
        if part >= sections.len() {
            return Err(format!("{id}: substage has {} parts", sections.len()));
        }
        let ids: Vec<StageId> = (0..sections.len()).map(|p| id.with_part(p as u8)).collect();
        let mut pending_stages = ids
            .iter()
            .map(|&p| source.stage_assets(rom, p).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        let mut pending_combat = ids
            .iter()
            .map(|&p| source.combat(rom, p).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        let scroll = ids
            .iter()
            .map(|&p| source.scroll_model(rom, p))
            .collect::<Result<Vec<_>, _>>()?;
        let bounds = sections
            .iter()
            .map(|section| source.camera_bounds(rom, section))
            .collect::<Result<Vec<_>, _>>()?;
        let assets = pending_stages[part].take().unwrap();
        let combat = pending_combat[part].take().unwrap();
        let sprites = PlayerAssets::load(rom)?;
        let music = source.start_music(rom, id)?;
        // $11720/$11754: the part record's player and camera start.
        let (x, y) = sections[part].player_start;
        let mut player = Player::new(x, y);
        player.grounded = Player::occupied(&assets, x, y + 1);
        (player.camera_x, player.camera_y) = sections[part].camera_start;
        let special = crate::special::Special::for_stage(rom, id, &sections[part], &mut player);
        let objects = source.objects(rom, id)?;
        let autoscroll = CameraAutoScroll::new(
            AutoScroll(sections[part].raw[13]),
            (player.camera_x, player.camera_y),
        );
        let mut stage = Viewer::new(assets, View::Scene);
        // The asset preview uses the final mech colours; gameplay begins black.
        if id.round == 5 && id.substage == 3 {
            stage.reload_boss_planes();
        }
        let door = doors::Door::entrance(rom, id, &sections[part])?;
        let mut game = Self {
            source,
            practice: None,
            run: crate::profile::Run {
                initial_ammo: crate::options::GameOptions::default().start_ammo(rom),
                round_start_ammo: crate::options::GameOptions::default().start_ammo(rom),
                ..Default::default()
            },
            stage_id: id,
            complete: false,
            requests: 0,
            part_request: None,
            game_flags: 0,
            pad: BossPad::default(),
            object_signals: Vec::new(),
            transitions: 0,
            sections,
            scroll,
            bounds,
            autoscroll,
            background: BackgroundY::at(0),
            rom: rom.into(),
            pending_stages,
            combat,
            pending_combat,
            cues: Vec::new(),
            music,
            music_request: Some(music),
            player,
            stage,
            sprites,
            cast: None,
            arena: None,
            special,
            markers: markers::Markers::default(),
            objects,
            high_score: 10_000,
            round_flags: RoundFlags::default(),
            fade: None,
            font: Font::load(rom)?,
            hud_renderer: HudRenderer::load(rom)?,
            // Direct play (`--play`) uses the power-on options: NORMAL.
            options: crate::options::GameOptions::default(),
            continues: crate::options::GameOptions::default().continues(rom),
            demo_counter: 0,
            extra_life: crate::options::GameOptions::default()
                .extra_life_step(rom)
                .map(|step| (step, step)),
            death: None,
            lag: 0,
            lost_fade: None,
            part_fade: None,
            door,
        };
        game.enter_markers();
        game.apply_camera_model();
        game.background = BackgroundY::at(game.player.camera_y);
        game.sync_stage_camera();
        Ok(game)
    }
}
