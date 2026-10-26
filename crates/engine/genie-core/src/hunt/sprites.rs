//! The hunt's sprites: standing, walking and the task's; an asked drop-off and the kind of
//! what a unit carries.

use crate::attack::set_sprite;
use crate::hunt::HuntTask;
use crate::move_to::MoveTo;
use crate::sprite::Graphic;

/// `FUN_140ac5210`, the sprite a hunter stands with: the task's carrying sprite while it holds a
/// whole unit or more of the task's resource and the sprite exists, else its master's standing
/// sprite (`standing`).
pub(crate) fn stand_sprite(
    task: &HuntTask,
    standing: i16,
    held_kind: i16,
    held: f32,
    graphics: &std::collections::BTreeMap<i16, Graphic>,
) -> Option<Graphic> {
    let g = |id: i16| (0 <= id).then(|| graphics.get(&id).copied()).flatten();
    let carrying = task.sprites[3];
    if task.resource != -1
        && task.resource == held_kind
        && 1 <= held as i32
        && g(carrying).is_some()
    {
        g(carrying)
    } else {
        g(standing)
    }
}

/// Slot 0x78 (`FUN_140a78060`, the base action's; `UNIT_ORDER`'s drop-off) of an action whose
/// task takes `resource` (`+0x20`, any with −1) for a unit carrying some of `held_kind` (`+0x310`
/// above 0, the caller's test): a load of the task's resource (compared by [`resource_kind`])
/// asks the caller for `set_state(8)` (true); a load of another resource has the caller's unit
/// AI take notify 0x21c from and to the unit (its slot 0x38, [`crate::unit_ai::OTHER_LOAD`]) and
/// returns false.
pub(crate) fn asked_drop_off(resource: i16, held_kind: i16, attributes: usize) -> bool {
    resource == -1 || resource_kind(held_kind, attributes) == resource_kind(resource, attributes)
}

/// The player's resource of what a unit carries (`FUN_140fdd4f0`): the kinds 0 to 3, 9 and 56
/// themselves, the foods 15 to 17 food (0), any other none (−1); then 0 for a kind beyond the
/// player's `attributes` (`FUN_140efe5b0`).
pub(crate) fn resource_kind(kind: i16, attributes: usize) -> i16 {
    let kind = match kind {
        0..=3 | 9 | 56 => kind,
        15..=17 => 0,
        _ => -1,
    };
    if (0..attributes as i16).contains(&kind) {
        kind
    } else {
        0
    }
}

/// The hunt's sprites as graphics: the task's moving, working and carrying sprites, and the
/// master's standing sprite, which slot 0x68 takes for none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HuntSprites {
    /// The moving sprite.
    pub moving: Option<Graphic>,
    /// The working sprite.
    pub working: Option<Graphic>,
    /// The carrying sprite.
    pub carrying: Option<Graphic>,
    /// The master's standing sprite.
    pub standing: Option<Graphic>,
}

impl HuntTask {
    /// The hunt's view of master task `t`.
    #[must_use]
    pub const fn of(t: &crate::masters::Task) -> Self {
        Self {
            id: t.id,
            range: t.work_range,
            resource: t.attribute_type,
            sprites: t.sprites,
            work_value: t.work_val1,
            productivity: t.attribute_type2,
            kind4: t.attribute_type4,
        }
    }

    /// [`walk_sprite`]'s id from the task's sprites (`FUN_140ac51d0`), which the hunt's Attack
    /// takes for its `MoveTo` (`FUN_140a7c210`'s argument 4).
    #[must_use]
    pub const fn walk_sprite_id(&self, held_kind: i16, held: f32) -> i16 {
        let carrying = self.resource != -1 && held_kind == self.resource && 1 <= held as i32;
        if carrying && 0 <= self.sprites[3] {
            self.sprites[3]
        } else {
            self.sprites[0]
        }
    }
}

/// `FUN_140ac51d0`: the sprite a hunt's `MoveTo` shows (its `set_state(4)`, slot 0x68 with
/// `+0x58`): the task's carrying sprite while the unit carries at least 1 of the task's
/// resource, else its moving one; None for none.
#[must_use]
pub const fn walk_sprite(
    resource: i16,
    held_kind: i16,
    held: f32,
    sprites: &HuntSprites,
) -> Option<Graphic> {
    let carrying = resource != -1 && held_kind == resource && 1 <= held as i32;
    match (carrying, sprites.carrying) {
        (true, Some(g)) => Some(g),
        _ => sprites.moving,
    }
}

/// The unit's sprite, the Sprite stream and the sprite a walk shows: a `MoveTo` that turns to
/// state 4 shows its sprite (`0x140aa29e0`, slot 0x68 with `+0x58`).
#[derive(Debug)]
pub struct Shown<'a> {
    /// The unit's sprite.
    pub sprite: &'a mut Option<crate::sprite::ActiveSprite>,
    /// The Sprite stream.
    pub rng: &'a mut crate::rng::Well512,
    /// The `MoveTo`'s sprite.
    pub graphic: Option<Graphic>,
}

impl Shown<'_> {
    /// The `MoveTo` that started turned to state 4: the unit shows its sprite.
    pub(super) fn walking(&mut self, state: MoveTo) {
        if state == MoveTo::Moving
            && let Some(g) = self.graphic
        {
            set_sprite(self.sprite, g, self.rng);
        }
    }
}
