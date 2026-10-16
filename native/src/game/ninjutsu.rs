//! Ninjutsu selection and mode `$22A96`.
use super::Game;
use crate::sound_events::SoundEvent;

impl Game {
    /// Select a technique independently of the remaining casting stock.
    pub fn select_ninjutsu(&mut self, index: usize) -> bool {
        if index > 3 {
            return false;
        }
        self.combat.weapons.ninjutsu_selected = index as u8;
        true
    }

    /// One update in ninjutsu mode `$22A96`: enemies and the stage wait while
    /// the technique's handler acts at its recovered cursors. True when the
    /// update is `Ninjutsu_Finish` on foot, which the caller runs as the
    /// player's ordinary update.
    pub(super) fn tick_ninjutsu(&mut self) -> bool {
        use crate::ninjutsu::{ART_LOAD_LAG, Effect, FINISH_LAG, Kind, PROTECTION_TICKS, effects};
        let Some(mut cast) = self.cast.take() else {
            return false;
        };
        if cast.finishing {
            // $2192E..$2194C, the update after the end flag: play resumes
            // with 64 ticks of protection and the saved palette. The wait at
            // $21A06 costs the next frame.
            let words = self.stage.assets.palette_words;
            self.stage.assets.palettes = crate::render::cram_palettes(&words);
            self.stage.y = self.player.camera_y;
            if cast.kind == Kind::Fushin {
                self.player.fushin = true; // $2097A
            }
            self.lag = self.lag.max(FINISH_LAG);
            let Some(special) = &mut self.special else {
                // On foot the player's update enters the neutral state
                // ($219EC) without moving, as Player_Freeze leaves it; the
                // caller sets the protection after it.
                return true;
            };
            // Mounted, Player_EnterNeutral puts the rider back on the mount.
            // The update takes no buttons (`Player_Freeze`), and the next
            // one does not count the protection down (plr_flags bit 4).
            self.player.ticks += 1;
            self.combat.invulnerability = PROTECTION_TICKS + 1;
            special.resume_after_ninjutsu(&mut self.player);
            return false;
        }
        // Issue #56: the handler's effect objects and palette writes, then
        // lists 3 and 9 (which keep running through Kariu's wave).
        let (x, y) = self.player.position();
        let anchor = crate::ninjutsu_fx::Anchor {
            x,
            y,
            facing_left: self.player.facing_left,
            camera: (self.player.camera_x, self.player.camera_y),
        };
        let acting = (cast.hold == 0).then_some(self.player.animator.cursor);
        let words = self.stage.assets.palette_words;
        cast.fx.tick(&self.rom, cast.kind, acting, anchor, &words);
        self.stage.assets.palettes = cast.fx.palettes(&words);
        self.stage.y = self.player.camera_y + cast.fx.shake();
        if cast.hold > 0 {
            cast.hold -= 1;
            if cast.hold == 0 && cast.kind == Kind::Kariu {
                // $22A78: Mode_KariuWave ends with MUSIC RESTART.
                self.cue(SoundEvent::KariuMusicRestart);
            }
            self.cast = Some(cast);
            return false;
        }
        let camera = (self.player.camera_x, self.player.camera_y);
        for effect in effects(cast.kind, self.player.animator.cursor) {
            match *effect {
                Effect::Sound(event) => self.cue(event),
                Effect::Shield(hits) => self.combat.weapons.shield = hits,
                Effect::Strike(damage) => {
                    self.combat.strike_visible(camera, damage);
                }
                Effect::LifeCost => {
                    if self.combat.spend_mijin_life().lethal {
                        // $20740..$20768: state 42 with no spare life. The
                        // blast's tail ($2086E) restores the palette,
                        // Player_Freeze stops the body and Mode_Game
                        // returns; this update's stepper already moved the
                        // cursor to $20.
                        let words = self.stage.assets.palette_words;
                        self.stage.assets.palettes = crate::render::cram_palettes(&words);
                        self.stage.y = self.player.camera_y;
                        let death_state = if self.special.as_ref().is_some_and(|special| {
                            special.mount == crate::special::Mount::Surfboard
                        }) {
                            crate::movement::state::MIJIN_DEATH_SURF
                        } else {
                            crate::movement::state::MIJIN_DEATH
                        };
                        self.player.mijin_death(death_state);
                        self.start_death(death_state);
                        return false;
                    }
                }
                Effect::Hold(ticks) => cast.hold = ticks,
                Effect::ArtLoad => self.lag = self.lag.max(ART_LOAD_LAG),
            }
        }
        // The stepper sets the end flag; the handler sees it on the next
        // update (`btst #0,plr_status` first in every technique's state).
        cast.finishing = self.player.tick_frozen();
        self.cast = Some(cast);
        false
    }
    /// Write `$FF37E7` directly, as `Demo_Init` does (no pause-menu rule).
    pub fn set_ninjutsu(&mut self, index: usize) {
        self.combat.weapons.ninjutsu_selected = index.min(3) as u8;
    }
    /// Technique selected for the next A press (0 Ikazuchi .. 3 Mijin) and
    /// the uses left in this life.
    pub fn ninjutsu(&self) -> (usize, u8) {
        (
            usize::from(self.combat.weapons.ninjutsu_selected),
            self.combat.weapons.ninjutsu_uses,
        )
    }
}
