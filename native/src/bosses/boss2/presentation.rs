//! Flying-machine fragments, afterimages, palettes and screen wave.
use super::*;

impl Boss2 {
    /// `Func_02F146`: one explosion within 16 px of the boss.
    pub(super) fn explosion(cx: &mut Ctx, e: &Enemy) {
        let r = cx.random();
        let dx = i32::from((r >> 8) as u8 as i8) >> 3;
        let dy = i32::from(r as u8 as i8) >> 3;
        let first = cx.shared.puff_seq.first().copied().unwrap_or(0);
        cx.effect(Effect::new(EffectKind::Puff, e.x + dx, e.y + dy, first));
    }

    /// `Func_02BEE2`: six fragments fly up and fall, then eight flares
    /// spread and return, each after a random delay (the hit and the fall).
    pub(super) fn burst_pieces(&self, cx: &mut Ctx, e: &Enemy) {
        let (count, first, gravity, life) = self.pieces.debris;
        for _ in 0..count {
            // $26406: $25A + (r & 1); vx = low byte >> 6, vy = (high | $80) >> 5.
            let r = cx.random();
            let mut piece = Effect::new(EffectKind::Pieces, e.x, e.y, first + (r & 1));
            piece.motion = Motion {
                vx: (i32::from(r as u8 as i8) >> 6) << 16,
                vy: (i32::from(((r >> 8) as u8 | 0x80) as i8) >> 5) << 16,
                gravity,
                ..Motion::default()
            };
            piece.counter = life - 1;
            cx.effect(piece);
        }
        let (count, first, frames, motions) = self.pieces.flares;
        for index in (0..count).rev() {
            let delay = (cx.random() & self.pieces.delay_mask) + 1;
            let mut flare = Effect::new(EffectKind::Flare, e.x, e.y, first);
            flare.delay = delay;
            flare.motion = motions[index % motions.len()];
            flare.period = self.pieces.flare_period;
            flare.counter = i32::from(flare.period);
            flare.phase = frames;
            cx.effect(flare);
        }
    }
    /// `Func_02BCDA`: a streak of the machine at (x, y), `vx` px a tick.
    pub(super) fn streak(&self, cx: &mut Ctx, x: i32, y: i32, vx: i32) {
        let (mapping, life, ..) = self.pieces.streak;
        let mut streak = Effect::new(EffectKind::Pieces, x, y, mapping);
        streak.motion.vx = vx << 16;
        streak.counter = life - 1;
        cx.effect(streak);
    }
    /// `Func_02BF68`: a still list-9 afterimage behind every object.
    /// It links the record at the head of list 9 (`$2BF7C`), so the newest
    /// afterimage is the first list-9 sprite.
    pub(super) fn trail(&self, cx: &mut Ctx, e: &Enemy, (mapping, life): (u16, i32)) {
        let mut ghost = Effect::new(EffectKind::Pieces, e.x, e.y, mapping);
        ghost.base = self.pieces.trail_tile;
        ghost.back = true;
        ghost.head = true;
        ghost.counter = life - 1;
        cx.effect(ghost);
    }
    /// `$2B6C0`: a spark circling in toward the orb, from the angle and
    /// step in the random word (`$FF001C`, read without advancing it).
    ///
    /// Nothing advances the generator during the charge, but the main
    /// loop turns it (`rol.w $FF001C`, `Main_WaitFrame` `$512`) until each
    /// VBlank: Genesis Plus GX, 2-3, 600 to 1,560 turns a frame, so every
    /// spark reads a new rotation and they close in from all around the
    /// orb. The idle time is not modelled; the charge's frame count stands
    /// in for the turns (mod 16), on a copy, so the shared sequence the
    /// other handlers draw from is unchanged.
    pub(super) fn orbit(&self, cx: &mut Ctx, e: &Enemy) {
        let (tile, dy, life, first) = self.pieces.orbit;
        let r = crate::ninjutsu_fx::orbit_random(*cx.random, (e.counter >> 2) as u64);
        let mut spark = Effect::new(EffectKind::Orbit, e.x, e.y, first + (life >> 3) as u16);
        spark.base = tile;
        spark.angle = r >> 4;
        spark.phase = (r & 0xf) as u8;
        spark.counter = life;
        spark.origin = (e.x, e.y + dy);
        // Placed by its first update, in this frame's effect pass.
        cx.effect(spark);
    }
    /// `$2B942`: the gun's shell casing, bouncing on the floor until slow.
    pub(super) fn casing(&self, cx: &mut Ctx, e: &Enemy) {
        let (mapping, dy, vx, vy, gravity) = self.pieces.casing;
        let mut casing = Effect::new(EffectKind::Pieces, e.x, e.y + dy, mapping);
        casing.base = self.pieces.casing_tile;
        casing.motion = Motion {
            vx: vx << 16,
            vy: vy << 16,
            gravity,
            ..Motion::default()
        };
        // $26574: it halves on each landing and is freed at the third.
        casing.phase = 3;
        casing.counter = i32::from(u16::MAX);
        cx.effect(casing);
    }

    /// Line 1 as `Func_02BF46` last wrote it into the `$FF16FA` fade
    /// target: the stage's until the first hit, then `Data_02AFA4` by the
    /// health before each hit (the native health after it).
    fn health_line(&self, e: &Enemy) -> Option<[u16; 16]> {
        (i32::from(e.health) <= self.health)
            .then(|| self.health_palette.colors(usize::from(e.health)))
    }
    /// `$2B314`: the whole `$FF16FA` target back into CRAM.
    pub(super) fn reload_palette(&self, cx: &mut Ctx, e: &Enemy) {
        for line in 0..4 {
            match self
                .health_line(e)
                .filter(|_| line == self.health_palette.line)
            {
                Some(colors) => cx.palette_op(PaletteOp::Load(line, colors)),
                None => cx.palette_op(PaletteOp::Reload(line)),
            }
        }
    }
    /// `Func_0006E8` with d7 = `$3F`: one step of all 64 colours toward it.
    pub(super) fn fade_palette(&self, cx: &mut Ctx, e: &Enemy) {
        for line in 0..4 {
            match self
                .health_line(e)
                .filter(|_| line == self.health_palette.line)
            {
                Some(colors) => cx.fade_palette(line, colors),
                None => cx.restore_palette(line),
            }
        }
    }
    /// `$2B77A`: every CRAM colour white.
    pub(super) fn white_flash(&self, cx: &mut Ctx) {
        for line in 0..4 {
            cx.fill_palette(line, self.white);
        }
    }
    /// `Func_02BD3C`: line 2 colour 9's green falls every fourth tick
    /// (`(-$3A << 3) & $E0` into `$FFF052`), and every fourth tick line 3
    /// colours 5..11 and 12..13 step through their tables: the walls pulse
    /// and the lamps blink.
    pub(super) fn cycle_palette(&self, cx: &mut Ctx, e: &Enemy) {
        let t = e.counter;
        cx.palette_op(PaletteOp::Color(2, 9, (t.wrapping_neg() << 3) & 0xe0));
        if t & 3 != 0 {
            return;
        }
        let pair = self.cycle.pairs[usize::from((t & 0x1c) >> 2)];
        let row = self.cycle.rows[usize::from(self.cycle.order[usize::from((t >> 2) & 0xf)])];
        let mut words = [pair[0]; 8];
        words[..7].copy_from_slice(&row);
        cx.palette_op(PaletteOp::Words(3, 5, words));
        cx.palette_op(PaletteOp::Color(3, 13, pair[1]));
    }
    /// `Func_02B832`: this tick's wave, then its phase and amplitude grow.
    /// Its `Game_Flags` bit 7 makes the pad read Left and Right swapped.
    pub(super) fn wave(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.signal(Signal::SwapLeftRight);
        self.draw_wave(cx, e);
        e.vars[P2E] = (e.vars[P2E] + self.wave.growth).min(self.wave.cap);
    }
    /// `$2BB4E`: the wave dies down after the second flash, the pad normal.
    pub(super) fn calm(&self, cx: &mut Ctx, e: &mut Enemy) {
        self.draw_wave(cx, e);
        e.vars[P2E] = (e.vars[P2E] - self.wave.growth).max(0);
    }
    fn draw_wave(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.plane_op(PlaneOp::Wave {
            amplitude: e.vars[P2E] as u16,
            phase: e.vars[P30] as u16,
        });
        e.vars[P30] = (e.vars[P30] + self.wave.phase_step) & 0xffff;
    }
}
