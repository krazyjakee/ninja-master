//! Type `$40`, `Enemy40` (`$3616A`): the list-7 item scheduler.
use crate::{
    combat::{Enemy, Signal, Spawn},
    enemies::{Behavior, Ctx, HitResponse},
    game::Terrain,
    rom::{Result, byte, pointer, signed, word},
    stage_objects::Drop,
};

/// `+$32`: the schedule (native: its index) and the entry reached.
const SCHEDULE: usize = 0;
const ENTRY: usize = 1;

/// One `$3629C` schedule entry (`$3618C`): an X word whose bit 15 is the
/// item's sprite priority, then the contents and the Y byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Scheduled {
    pub(crate) x: i32,
    pub(crate) priority: bool,
    pub(crate) kind: u8,
    pub(crate) y: i32,
}

/// `$40`: the invisible pickup scheduler of 2-1 and 4-1. The spawn flags
/// (`& $FFF`, a byte offset into the `$3629C` longs) select a schedule.
/// Once the camera X passes an entry's X, one list-7 item (`$361D2`)
/// enters 336 px further right, flying left at 5 px/tick from routine 4,
/// one entry per frame. The end marker frees the object (`$129B8`).
pub(crate) struct PickupScheduler {
    pub(crate) schedules: Vec<Vec<Scheduled>>,
    /// `$361B6`: `addi.w #$150,d5`.
    lead: i32,
    /// `$36206`: `move.w #-5,$18(a6)`, the integer word of the X velocity.
    speed: i32,
}
impl PickupScheduler {
    fn drop(&self, entry: &Scheduled) -> Drop {
        Drop {
            kind: entry.kind,
            x: entry.x + self.lead,
            y: entry.y,
            vx: self.speed,
            // $3619C / $361EE: the entry's own sprite priority bit.
            high_priority: entry.priority,
        }
    }

    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        // Only the first two longs are schedules (2-1 and 4-1 select
        // offsets 0 and 4); the words after them are colour data.
        let mut schedules = Vec::new();
        for i in 0..2 {
            let mut p = pointer(rom, 0x3629c + i * 4)?;
            let mut list = Vec::new();
            loop {
                let x = word(rom, p)?;
                if x == 0xffff {
                    break;
                }
                if list.len() == 64 {
                    return Err("unterminated $3629C schedule".into());
                }
                list.push(Scheduled {
                    x: i32::from(x & 0x7fff),
                    priority: x & 0x8000 != 0,
                    kind: byte(rom, p + 2)?,
                    y: i32::from(byte(rom, p + 3)?),
                });
                p += 4;
            }
            schedules.push(list);
        }
        Ok(Self {
            schedules,
            lead: i32::from(word(rom, 0x361b8)?),
            speed: signed(rom, 0x36208)? << 16,
        })
    }
}
impl Behavior for PickupScheduler {
    fn label(&self, _kind: u16) -> String {
        "Enemy40 $3616A (list-7 pickup scheduler)".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, _terrain: &dyn Terrain) {
        if e.routine == 0 {
            // $36172.
            e.vars[SCHEDULE] = i32::from(spawn.flags & 0xfff) / 4;
            e.vars[ENTRY] = 0;
            e.routine = 4;
        }
        let entry = usize::try_from(e.vars[SCHEDULE])
            .ok()
            .and_then(|schedule| self.schedules.get(schedule))
            .and_then(|list| list.get(usize::try_from(e.vars[ENTRY]).ok()?));
        match entry {
            // $361CA: the end marker.
            None => Ctx::free(e),
            // $361A4: `cmp.w Camera_X,d5` / `bpl` waits while X >= camera.
            Some(entry) if entry.x < cx.camera.0 => {
                cx.signal(Signal::Drop(self.drop(entry)));
                e.vars[ENTRY] += 1;
            }
            Some(_) => {}
        }
    }
    fn approaching_drops(
        &self,
        enemy: &Enemy,
        spawn: &Spawn,
        camera_fixed: i32,
        step: i32,
    ) -> Vec<Drop> {
        if step <= 0 {
            return Vec::new();
        }
        let (schedule, entry) = if enemy.routine == 0 {
            (i32::from(spawn.flags & 0xfff) / 4, 0)
        } else {
            (enemy.vars[SCHEDULE], enemy.vars[ENTRY])
        };
        let Some(entries) = usize::try_from(schedule)
            .ok()
            .and_then(|i| self.schedules.get(i))
            .and_then(|list| list.get(usize::try_from(entry).ok()?..))
        else {
            return Vec::new();
        };
        let mut earliest = 1;
        entries
            .iter()
            .map(|entry| {
                // Strict X < camera, with the mounted camera in 8.8 units.
                // At most one entry is dispatched each update, even if a
                // camera jump has left several entries ready at once.
                let remaining = (i64::from(entry.x + 1) << 8) - i64::from(camera_fixed);
                let ahead = (remaining.max(0) as u64)
                    .div_ceil(step as u64)
                    .max(earliest);
                earliest = ahead + 1;
                let mut drop = self.drop(entry);
                // Allocation is followed by the pickup's first movement
                // in the same update. Join that sprite on the next frame.
                drop.x += (1 - ahead as i32) * (self.speed >> 16);
                drop
            })
            .collect()
    }
    fn hit(&self, _enemy: &Enemy, _kind: u16) -> HitResponse {
        HitResponse::Pass
    }
}
