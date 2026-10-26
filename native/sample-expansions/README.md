# Expansion packs

Copy a folder from here (or a ZIP of one) into the `expansions` folder beside
the executable to play it from **EXPANSIONS** on the main menu. The
`NINJA_MASTER_EXPANSIONS` environment variable can point to another folder.
When running from a source checkout, the folder is `expansions/` at the
repository root.

`forest-trail` is a sample: a new path through the forest on round 1's art,
with a new enemy (the leaping monk), then a rematch in the first boss's arena.

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

Consecutive parts hand the player over like their base parts do. For a part
built on a different base, a substage of its own is usually simpler.

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
