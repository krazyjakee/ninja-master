//! `$37D1C`: the list-1 `$0B` chain forming the MIJIN charge pillar.
//! Each section rises to 32 pixels above its parent, spawns the next,
//! then waits. Releasing the bottom section drains the chain upward.
use super::*;
use crate::combat::{Effect, EffectKind};

const ROOT: u16 = 2;
const RELEASE: u16 = 4;

impl Boss7 {
    pub(super) fn start_pillar(&self, cx: &mut Ctx, e: &Enemy) {
        let mut root = self.pillar_section((e.x, e.y), 0);
        root.flags = ROOT;
        cx.effect(root);
    }

    pub(super) fn release_pillar(cx: &mut Ctx) {
        if let Some(root) = cx
            .effects
            .iter_mut()
            .find(|p| p.kind == EffectKind::Boss7Pillar && p.angle == 0)
        {
            root.flags |= RELEASE;
        }
    }

    fn pillar_section(&self, at: (i32, i32), index: u16) -> Effect {
        let mut section = Effect::new(EffectKind::Boss7Pillar, at.0, at.1, 0);
        section.origin = at;
        section.base = 0x2000;
        section.angle = index;
        section
    }

    /// The list-1 pass runs newly appended children in the allocating
    /// frame. Keep the original phase offsets and integration order.
    pub(super) fn pillar(&self, cx: &mut Ctx, e: &Enemy) {
        let mut i = 0;
        while i < cx.effects.len() {
            if cx.effects[i].kind != EffectKind::Boss7Pillar || !cx.effects[i].alive {
                i += 1;
                continue;
            }
            let mut child = None;
            let mut release = None;
            let section = &mut cx.effects[i];
            let anchor = section.origin.1
                - if section.flags & ROOT != 0 {
                    0
                } else {
                    self.pillar_spacing
                };
            let mut animate = true;
            match section.phase {
                0 => {
                    let (x, y) = (section.x - cx.camera.0, section.y - cx.camera.1);
                    if x <= -16 || x > 336 || y <= -16 || y > 240 {
                        section.alive = false;
                    } else if section.flags & ROOT != 0 {
                        section.phase = 8;
                    } else {
                        section.phase = 4;
                        section.motion.vy = self.pillar_velocity;
                    }
                    // The first routine returns with mapping zero.
                    animate = false;
                }
                4 => {
                    if section.y <= anchor {
                        section.y = anchor;
                        section.motion.vy = 0;
                        // $37D98 falls through $37DB2 in this same update.
                        child =
                            Some(self.pillar_section((section.x, section.y), section.angle + 1));
                        section.phase = 12;
                    }
                }
                8 => {
                    child = Some(self.pillar_section((section.x, section.y), section.angle + 1));
                    section.phase = 12;
                }
                12 => {}
                _ => {
                    if section.y <= anchor - self.pillar_spacing {
                        release = Some((section.origin.0, section.angle + 1));
                        section.alive = false;
                    }
                }
            }
            if section.phase == 12 && section.flags & RELEASE != 0 {
                section.flags &= !RELEASE;
                section.phase = 16;
                section.motion.vy = self.pillar_velocity;
            }
            if animate {
                section.mapping = if e.age & 1 == 0 { self.pillar_frame } else { 0 };
            }
            if section.alive {
                section.motion.step(&mut section.x, &mut section.y);
            }
            if let Some(child) = child {
                cx.effect(child);
            }
            if let Some((x, next)) = release
                && let Some(child) = cx.effects.iter_mut().find(|p| {
                    p.kind == EffectKind::Boss7Pillar && p.origin.0 == x && p.angle == next
                })
            {
                child.flags |= RELEASE;
            }
            i += 1;
        }
    }
}
