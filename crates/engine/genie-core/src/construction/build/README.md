# `construction/build`: the Build action

A builder walks to its foundation with a `MoveTo` sub-action (state 4). At the foundation it faces
it and joins the work (state 7). A builder that stands on the footprint of a foundation that no
one started walks off it first (state 223) and comes back. Each update of work adds to the
foundation's construction. The first work starts the construction, and the work that reaches the
build time completes the building.

- `mod.rs` holds the action's state, the construction's state and their constants.
- `update.rs` starts the action and runs its state in each update.
- `states.rs` holds the states that the update and the unit's stop set.
- `walk_ends.rs` holds the ends of the walk: arrived, failed, or the foundation lost.
- `placing.rs` holds the placement on a foundation that no one started, and the walk off the
  footprint.
- `construction.rs` holds a foundation's construction: its work, its start and its completion.
- `clearing.rs` removes the objects on a new building's footprint.
- `drop_off.rs` sends a builder that carries resources to a drop site first.
- `more_work.rs` finds a resource near the finished building for the builder.
- `turn_entries.rs` holds the construction's parts that run on the whole world.
- `unit_ai_side.rs` holds what the unit AI asks of a build.

## In the game

- `TRIBE_Action_Build` (action type 101, `tact_bld.cpp`).

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`clearing.rs`](clearing.rs) | The footprint clearing (`FUN_1415c3620`) of a building: the objects on its tiles that go. |
| [`construction.rs`](construction.rs) | A foundation's construction (`Construction`): its work, its start and its completion, with the completion's storage, ground and counts. |
| [`drop_off.rs`](drop_off.rs) | State 8: a builder that carries something walks to a drop site first (`DropSearch`, `DropStart`). |
| [`mod.rs`](mod.rs) | A builder's Build action (`TRIBE_Action_Build`, type 101, vtable `0x142ed6628`) up to the builder's walk: the action starts in state 4 with a `MoveTo` sub-action (`RGE_Action_Move_To`, type 1) toward the foundation; a builder already at the foundation works in its first update (state 6). |
| [`more_work.rs`](more_work.rs) | A builder's search for more work once its Build ended without queued orders (the Build's case 0xdf): a resource around the building it finished, as the unit AI's `ClosestResourceObject` names it. |
| [`placing.rs`](placing.rs) | The placement on a foundation no builder started (state 7), a builder's joining and its work, the walk off the footprint (state 223) and back. |
| [`states.rs`](states.rs) | The states the update and the unit's stop set: the prelude, the work's end and what follows it, the restart, the idle, a retarget, the halt and the end. |
| [`tests.rs`](tests.rs) | Tests. |
| [`turn_entries.rs`](turn_entries.rs) | The construction's entries outside a builder's update (`crate::world::World`): a building's work and completion, its storage's leave, the built object's stacking and its new objects. |
| [`unit_ai_side.rs`](unit_ai_side.rs) | What the unit AI asks of a build while it runs (through its view, `UnitView`): the build command at once, the builder's search for another foundation, and a unit's spot off a foundation's footprint. |
| [`update.rs`](update.rs) | The Build action's start (state 4 toward the foundation) and its update, which runs the action's state; the AI's build orders whose foundation is gone. |
| [`walk_ends.rs`](walk_ends.rs) | The ends of the builder's walk: at its end, failed, its foundation lost; the walk to a point (state 11). |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`combat`](../../combat/), [`commands`](../../commands/), [`garrison`](../../garrison/), [`gathering`](../../gathering/), [`map`](../../map/), [`math`](../../math/), [`movement`](../../movement/), [`objects`](../../objects/), [`orders`](../../orders/), [`pathing`](../../pathing/), [`players`](../../players/), [`production`](../../production/), [`unit_ai`](../../unit_ai/), [`world`](../../world/).
<!-- /uses -->
