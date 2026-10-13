//! Boss palette effects shared by the round handlers (#92): a CRAM line
//! chosen by the boss's health, the list-1 `$37` hit flasher that shows it,
//! and the `$6E8` fade of several lines.
use crate::{
    combat::PaletteOp,
    enemies::Ctx,
    rom::{Result, pointer, word, words},
};

/// `Data_0362A4`: the `$37` flasher's tables, each a header word (rows - 1,
/// also the initial `+$43`) and 32-byte rows. Table 0 is one white row.
const FLASHER_TABLES: usize = 0x362a4;

/// A boss's health palette: whole CRAM lines indexed by its health
/// counter, clamped to the last entry (Boss2's `Data_02AFA4`, Boss3's
/// `Data_042062` + `Data_02BFCA`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HealthPalette {
    /// The CRAM line the handler writes.
    pub(crate) line: u8,
    entries: Vec<[u16; 16]>,
}

impl HealthPalette {
    /// Entries read from the ROM addresses `sources` yields, in health order.
    pub(crate) fn load(
        rom: &[u8],
        line: u8,
        sources: impl IntoIterator<Item = Result<usize>>,
    ) -> Result<Self> {
        let entries = sources
            .into_iter()
            .map(|source| words(rom, source?))
            .collect::<Result<Vec<_>>>()?;
        if entries.is_empty() {
            return Err("empty boss health palette".into());
        }
        Ok(Self { line, entries })
    }
    /// `count` consecutive 32-byte lines from `table`.
    pub(crate) fn table(rom: &[u8], line: u8, table: usize, count: usize) -> Result<Self> {
        Self::load(rom, line, (0..count).map(|i| Ok(table + i * 32)))
    }
    /// `Data_0362A4[index]`: a flasher table, drawn into line 1
    /// (`Func_038D6C` writes `$FFF020`).
    pub(crate) fn flasher(rom: &[u8], index: usize) -> Result<Self> {
        let table = pointer(rom, FLASHER_TABLES + index * 4)?;
        let rows = usize::from(word(rom, table)?) + 1;
        Self::table(rom, 1, table + 2, rows)
    }
    /// The colours for health counter `health`.
    pub(crate) fn colors(&self, health: usize) -> [u16; 16] {
        self.entries[health.min(self.entries.len() - 1)]
    }
    /// Copy the line for `health` into CRAM.
    pub(crate) fn show(&self, cx: &mut Ctx, health: usize) {
        cx.palette_op(PaletteOp::Load(self.line, self.colors(health)));
    }
    /// One `$6E8` step of the line toward the colours for `health`.
    pub(crate) fn fade(&self, cx: &mut Ctx, health: usize) {
        cx.fade_palette(self.line, self.colors(health));
    }
}

/// The list-1 `$37` palette flasher (`Func_038CEE`) a boss allocates on a
/// hit, kept in one scratch word of the boss (0 when idle; `$2C` count in
/// the high half, `$2A` duration in the low). Each update after the one
/// that starts it, odd counts show table 0's white row and even counts the
/// boss's row (its `+$43`, read live); at the duration it shows the row
/// and frees itself.
pub(crate) struct Flasher {
    white: [u16; 16],
}

impl Flasher {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            white: HealthPalette::flasher(rom, 0)?.colors(0),
        })
    }
    /// Allocate the flasher for `duration` updates (`$2A`).
    pub(crate) fn start(state: &mut i32, duration: u16) {
        *state = i32::from(duration);
    }
    /// One flasher update showing `rows` row `row` (a no-op when idle).
    pub(crate) fn step(&self, cx: &mut Ctx, state: &mut i32, rows: &HealthPalette, row: usize) {
        if *state == 0 {
            return;
        }
        let duration = *state & 0xffff;
        let count = (*state >> 16) + 1;
        if count == duration || count & 1 == 0 {
            rows.show(cx, row);
        } else {
            cx.palette_op(PaletteOp::Load(rows.line, self.white));
        }
        *state = if count == duration {
            0
        } else {
            (count << 16) | duration
        };
    }
}

/// One `$6E8` step of consecutive lines from `first` toward `targets`.
pub(crate) fn fade_lines(cx: &mut Ctx, first: u8, targets: &[[u16; 16]]) {
    for (line, &target) in (first..).zip(targets) {
        cx.fade_palette(line, target);
    }
}

/// One `$75C` step of each of `lines` toward white.
pub(crate) fn fade_white_lines(cx: &mut Ctx, lines: impl IntoIterator<Item = u8>) {
    for line in lines {
        cx.fade_white(line);
    }
}
