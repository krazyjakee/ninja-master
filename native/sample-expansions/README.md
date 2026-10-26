# Expansion packs

Copy a folder from here (or a ZIP of one) into the `expansions` folder beside
the executable to play it from **EXPANSIONS** on the main menu. The
`NINJA_MASTER_EXPANSIONS` environment variable can point to another folder.
When running from a source checkout, the folder is `expansions/` at the
repository root.

Two samples:

- `forest-trail`: a new path through the forest on round 1's art, with a new
  enemy (the leaping monk), then a rematch in the first boss's arena.
- `hangar-run`: a short warehouse run built from round 2-2's walkways, supports
  and pillars, with new art of its own: cargo crates and oil drums to climb,
  hazard signs, hanging lamps, wall vents and control panels. `generate.py`
  builds its layout. `paint.py` paints the crate and sign and adds
  `props.png`, the props Codex CLI drew from a written brief with
  `paint_props.py`.

## The editor

Choose **EDITOR** on the main menu, or run
`ninja-master-native --editor [PACK]`, to open the expansion editor in its own
window. From the command line it opens a pack folder, or starts a new pack. It writes the format below, so packs can still
be edited by hand. Opening a pack and saving it changes only the manifest's
formatting.

- **Pack** (left): the pack's id, name, description and code, and its rounds,
  stages and parts. Select a part to edit it. Add, remove and reorder parts,
  stages and rounds here.
- **Part** (right): the base and music, the layout's width, the tools, the
  metatile palette, the start and exit, the enemy list and the art cells.
- **Stage** (middle): the part as the game draws it. **G** toggles the grid,
  **C** the collision colours, **B** the background. Middle-drag or use the
  arrows to pan, the wheel to scroll, and Ctrl+wheel to zoom.

The tools are **Paint** (1) and **Rectangle** (2) for the layout, where the
right button picks up the metatile under it. **Enemies** (3) places and drags
enemies, and the right button deletes them. **Start** (4) and **Exit** (5)
move the start and exit. Ctrl+Z and Ctrl+Y undo and redo, and Ctrl+S saves.

**Pack > Check for problems** loads every part as the game would and lists
what stops one from loading. **F5** playtests the selected part in a separate
game window, and **F6** playtests from the start. Both use unsaved changes.
The editor picks legend characters for you. A new layout starts blank: the
editor never copies the base's layout into a pack.

## `expansion.json`

```json
{
  "version": 1,
  "id": "forest-trail",
  "name": "Forest Trail",
  "description": "A new path through the forest, then a rematch.",
  "rounds": [
    {
      "name": "The Hidden Trail",
      "substages": [
        { "parts": [ { "base": "1-1:0", "layout": "trail.txt", "legend": { ".": "00" } } ] },
        { "parts": [ { "base": "1-3:0" } ] }
      ]
    }
  ]
}
```

- `id` identifies the expansion: letters, digits, `-` and `_`. `name` is
  listed on the menu; `description` is optional.
- `code` (optional) names behaviour compiled into the game for this
  expansion: new enemy types and bosses. See [Code](#code).
- `rounds` play in order, and so do each round's `substages` and their
  `parts`. Stages are numbered from 1-1 within the expansion. Each round's
  `name` appears on its round card.

Each part is built on an original part, its **`base`** (`"R-S:P"`, with
the part counted from 0). The base supplies the art, palettes, metatiles,
collision, scrolling, camera limits, music, enemy types with art, any boss,
and stage behaviour such as horse riding or the round 7 door. Boss arenas end
when the boss is defeated.

The other fields of a part are optional:

| Field | Meaning |
| --- | --- |
| `layout` | A text file of 32 rows, one character per 16-pixel metatile column, 21 to 256 columns wide. Replaces the base's layout and drops its stage objects and "GO" marker. The camera stops at the layout's right edge. |
| `legend` | Each layout character's metatile number, as two hex digits. Metatile numbers belong to the base stage's tile set. |
| `start` | The player's start `[x, y]` in pixels, at the feet. Row 31's top is y = 496. |
| `exit` | The exit's x position in pixels. Reaching it enters the next part, or the next substage after the last part. |
| `enemies` | The part's enemies: `{"kind": "14", "x": 400, "y": 496, "flags": "0011"}`. `kind` is an original object type in hex, or one of the expansion's code's types. Its art must be loaded by the base part. `flags` is the spawn flags word. Without a list, a part keeps its base's enemies, unless it has its own layout. |
| `music` | The original part (`"R-S:P"`) whose music plays instead of the base's. |
| `art` | New metatiles for the layout, painted on a PNG sheet. See [Art](#art). |

Consecutive parts hand the player over like their base parts do. For a part
built on a different base, a substage of its own is usually simpler.

## Art

A part with its own layout can add metatiles painted on a PNG sheet of 16x16
cells:

```json
"art": {
  "sheet": "art.png",
  "cells": {
    "<": { "at": [0, 0], "collision": "solid" },
    "[": { "at": [2, 0] },
    "~": { "at": [3, 0], "collision": "platform", "front": true }
  }
}
```

Each key is a layout character, used like a legend entry; `at` is the
cell's column and row in the sheet. Cells take metatile numbers the
legend leaves free.

The art must follow the Mega Drive's rules, which keeps it at home beside the
original art:

- Each 8x8 quarter of a cell may only use colours from one of the base
  stage's four palette lines (16 colours each, shared with sprites).
  Transparent pixels show the background, like the cartridge's colour 0.
- Pick colours from the template's `palettes.png`; nearby values are snapped
  to the Mega Drive's 8 levels per channel.
- New tiles fill the base's free pattern slots: roughly 190 to 330 tiles,
  depending on the stage. Identical 8x8 tiles are stored once.

`collision` is one of `none` (the default), `solid`, `platform` (one-way
floor), `ledge` (one-way floor that can be hung from), `hang-bar`, `spikes`,
`conveyor-left`, `conveyor-right`, `no-cling`, `wall-left`, `wall-right`,
`mire` or `splash`. `front` draws the cell in front of sprites.

To start, export a template for the base part from your ROM:

```sh
ninja-master-native --expansion-template 2-2:0 my-art [--rom FILE]
```

It writes `palettes.png` (the base's palette lines, one row per line,
colour 0 left blank), a blank `art.png` sheet, and `metatiles.png` with
`metatiles.txt`: every metatile the base uses and its number, for the
legend. The template comes from your cartridge; keep it out of packs you
share.

## Code

Packs are data only. New logic lives in the game's source, in
`native/src/expansion/code/`, and a pack opts in with `"code": "<name>"`.
An expansion's code implements `Code`:

- `enemies` returns new enemy types. Each has a kind from `200` up in steps
  of 4, the cartridge type whose loaded art it draws with, its score,
  contact damage and health, and a `Behavior`: the same handler interface the
  original enemies use. Parts place the new types by kind like any other.
- `boss` can replace the boss of a part whose base is a boss arena.

`forest-trail`'s code (`forest_trail.rs`) adds the leaping monk (kind `200`),
which walks at the player and leaps, using the komuso's art and poses.
