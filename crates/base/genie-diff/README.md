# genie-diff

Where two values of the same type differ, field by field. Each difference carries its path from
the values compared (`life.hp`, `actions[0].to[1]`, `garrison.len`) and both sides' text.

A type derives `Diff` (`#[derive(Diff)]`, from genie-diff-derive):

- A struct compares its fields by name. A tuple struct of one field is that field; one of more
  fields numbers them.
- An enum differs by its variant's name; the same variant compares its fields.
- A float compares by its bits: `0.0` and `-0.0` differ, a NaN equals the same NaN.
- `#[diff(unsaved)]` marks a field the game's save does not hold. `Scope::Saved` leaves it out
  (a state against one loaded from the game's save); `Scope::All` reads it (two engine runs).
- `#[diff(gap = "why")]` marks a known difference: the engine does not model the field yet.
  `Scope::Saved` leaves it out; `Scope::SavedWithGaps` and `Scope::All` read it. The reasons are
  the work list of what the comparison cannot check yet.
- `#[diff(eq)]` compares a field by its `PartialEq`, as one value.

genie's divergence tools compare the engine's objects with the game's saves through it
(FD-045-D14). It is a base crate: it depends only on its derive, and every crate may use it.
