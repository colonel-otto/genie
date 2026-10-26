# `map`: the terrain and where the objects stand

The map holds the terrain of each tile and the places of the world's objects. Movement, pathing,
vision and the unit AI's searches read it.

- `terrain.rs` holds the map's tiles: their terrain, the terrain to restore, the layer, and what
  the `.dat` file says of each terrain.
- `spatial/` holds where every live object is: its position, its radii, its footprint, and the
  record that the game's obstruction manager keeps for it. The grid of the records answers the
  queries: which objects are near a point, which block a walk, which a unit can see.

## In the game

- `RGE_Map` (`map.cpp`, `tmap.cpp`): the tiles, 0x40 bytes each.
- The obstruction manager: the records of the objects' shapes, in a grid of levels.

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`mod.rs`](mod.rs) | The map: the terrain of each tile and where the world's objects stand (their positions, radii, footprints and the grid that finds them). |
| [`terrain.rs`](terrain.rs) | The map's terrain per tile (`RGE_Map`'s tiles, 0x40 bytes each: `+0x9` the terrain, `+0xa` the terrain to restore, `+0xc` the layer) and what the `.dat` says of each terrain, as far as the slice reads them: a completed building's snow check and its foundation terrain (`FUN_1411dbb70`, `FUN_1411ddd90`). |
| [`spatial/`](spatial/) | Where the world's objects are, as movement reads them: each live object's position, radii and class, and the obstruction record the game keeps for it in its obstruction manager (`DAT_143f0a088`, queried by `FUN_141051900`). |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`construction`](../construction/), [`math`](../math/), [`movement`](../movement/), [`objects`](../objects/), [`pathing`](../pathing/), [`players`](../players/), [`unit_ai`](../unit_ai/), [`vision`](../vision/), [`world`](../world/).
- Used by: [`combat`](../combat/), [`commands`](../commands/), [`construction`](../construction/), [`garrison`](../garrison/), [`gathering`](../gathering/), [`groups`](../groups/), [`movement`](../movement/), [`objects`](../objects/), [`orders`](../orders/), [`ownership`](../ownership/), [`pathing`](../pathing/), [`players`](../players/), [`production`](../production/), [`technology`](../technology/), [`unit_ai`](../unit_ai/), [`vision`](../vision/), [`world`](../world/).
<!-- /uses -->
