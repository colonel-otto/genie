# `movement/move_to/install`: the install's tests

The tests of the install of a unit's own `MoveTo` (`../install.rs`, slot 0x808 with it): the
move to a point and to an object from the world (`FUN_14106d340`), and slot 0x340's and slot
0x348's moves from the unit's step and from the world. A unit in state 3 or later keeps no new
`MoveTo`: the list's add deletes it unstarted, and its teardown stops the unit and lets its
target go. The start measures its gap from the unit's master's radii and reads the top speed.
They take `world::tests`'s walking villager and its town centres.

## Files

<!-- files -->
| File | What it holds |
| --- | --- |
| [`tests.rs`](tests.rs) | Tests: The install of a unit's own `MoveTo` through slot 0x808: a unit in state 3 or later keeps none (its teardown stops the unit and lets the target go), and the start's gap and top speed. |
<!-- /files -->

## How it connects

<!-- uses -->
- Uses: no other system.
<!-- /uses -->
