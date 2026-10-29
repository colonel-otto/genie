# `movement/move_to`

The `MoveTo` action: a unit walks to a target object or a point. It runs in two roles:

- As a sub-action (`mod.rs`): another action (an Attack, a Build, a Gather, a Hunt, an Explore)
  holds its state and runs its start and its walk with `start_walk_or_give_up` and
  `walk_or_give_up`.
- As a unit's own top action (`action.rs`, `MoveToAction`): a moved or following unit's, a
  wandering animal's. It holds its own state, goal, range, wait and sprite.

The `MoveTo`'s `set_state` (`0x140aa29e0`) shows the unit's sprites: turning to state 4 its own
sprite (`+0x58`) before the motion step runs, so the top speed reads that sprite; its end
(states 1 and 13), unless it is quiet, the standing sprite. `Shown` and `Shows` carry the
unit's sprite and the Sprite stream to it.

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`action.rs`](action.rs) | The `MoveTo` as a unit's own top action (`MoveToAction`): its constructors, its start and its update. |
| [`mod.rs`](mod.rs) | The `MoveTo` action (`RGE_Action_Move_To`, type 1, vtable `0x142ed6030`): a unit walks to a target object or a point. |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: [`combat`](../../combat/), [`construction`](../../construction/), [`gathering`](../../gathering/), [`map`](../../map/), [`math`](../../math/), [`objects`](../../objects/), [`world`](../../world/).
<!-- /uses -->
