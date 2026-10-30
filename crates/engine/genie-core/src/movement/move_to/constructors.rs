//! The quiet sub-action `MoveTo`s an action makes and adds to its sub-action list: what their
//! constructors read of the unit ([`MoveToReads`]) and the object one walks to ([`WalkTo`]); to an
//! object (`FUN_140aa0df0`), to a point (`FUN_140aa0d30`) and to none.

use crate::GenieError;
use crate::movement::footprint_gap;
use crate::movement::move_to::{
    MoveTo, MoveToAction, MoveWait, NO_TARGET_GAP, SpriteReads, TargetMoved,
};
use crate::objects::masters::{Master, Masters};
use crate::objects::sprite::Graphic;
use crate::objects::unit::Unit;
use crate::world::World;
use std::collections::BTreeMap;

/// The last object state a `MoveTo` takes as its target (`FUN_140a7a230`: one above is refused,
/// its id −1).
const LAST_TAKEN: u8 = 6;

/// The point a `MoveTo` without a target walks to (`FUN_140aa0df0` leaves its zeroed `+0x38`).
const NO_TARGET_POINT: [f32; 3] = [0.0; 3];

/// What a new sub-action `MoveTo` reads of its unit: the sprites its constructor takes for none
/// (the defaults of the unit's master and terrain look, and the sprites' table), the unit's
/// object state (`+0x640`: its action list adds a sub-action only below 3, `FUN_140a977e0`), its
/// speed (slot 0x208: without one the `MoveTo` starts failed, `0x140aa2100`), and its place and
/// its master's radii, from which the gap to its target is measured (`FUN_14118f680`).
#[derive(Clone, Copy, Debug)]
pub struct MoveToReads<'a> {
    /// The sprites a `MoveTo` takes when its maker gives none.
    pub sprites: SpriteReads<'a>,
    /// The unit's object state.
    pub state: u8,
    /// The unit's speed.
    pub speed: f32,
    /// The unit's place.
    pub at: [f32; 3],
    /// Its master's x and y radii (`+0x5d8`, `+0x5dc`).
    pub ground: [f32; 2],
}

/// An object a `MoveTo` walks to: its id, its place, the unit's footprint gap to it
/// (`FUN_14118f680`) and its object state (`+0x640`), None where the world does not hold it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkTo {
    /// Its id.
    pub id: i32,
    /// Its place, the `MoveTo`'s point (`+0x38`).
    pub at: [f32; 3],
    /// The unit's footprint gap to it.
    pub gap: f32,
    /// Its object state.
    pub state: Option<u8>,
}

impl<'g> MoveToReads<'g> {
    /// What a new `MoveTo` reads of `unit` of `master` (its own, or the one its task switches it
    /// to), with the sprites' table `graphics`: an object without movement has no speed (a
    /// static object's slot 0x208).
    #[must_use]
    pub fn of(unit: &Unit, master: &Master, graphics: &'g BTreeMap<i16, Graphic>) -> Self {
        Self {
            sprites: SpriteReads::of(master, unit.abilities.look.as_ref(), graphics),
            state: unit.life.state,
            speed: unit.body.mover().map_or(0.0, |m| m.stats.speed),
            at: unit.position(),
            ground: master.radius,
        }
    }

    /// What a new `MoveTo` reads of `unit` with the sprites `sprites`: its state and speed, its place
    /// and the radii of its movement state (none for a unit that does not move).
    #[must_use]
    pub fn with_sprites(sprites: SpriteReads<'g>, unit: &Unit) -> Self {
        let mover = unit.body.mover();
        Self {
            sprites,
            state: unit.life.state,
            speed: mover.map_or(0.0, |m| m.stats.speed),
            at: unit.position(),
            ground: mover.map_or([0.0; 2], |m| [m.stats.radius[0], m.stats.radius[1]]),
        }
    }

    /// The object `id`, live or sleeping, as a `MoveTo` of this unit walks to it ([`WalkTo`]: its
    /// place, its state and the gap between their footprints by their masters' radii,
    /// `FUN_14118f680`), None where the world does not hold it.
    ///
    /// # Errors
    ///
    /// An object whose master its player's table lacks.
    pub fn walk_to(&self, w: &World, id: i32) -> Result<Option<WalkTo>, GenieError> {
        let Some((player, t)) = w.object(id) else {
            return Ok(None);
        };
        let at = t.position();
        let radius = t.master(&w.masters, player)?.radius;
        Ok(Some(WalkTo {
            id,
            at,
            gap: footprint_gap(self.at, self.ground, at, radius),
            state: Some(t.life.state),
        }))
    }

    /// A quiet `MoveTo` to the object `to` within `range` (`FUN_140aa0df0` with 1 and 1, then
    /// `FUN_140a7a170` with 1): its point the object's place, the sprite `given` (else the
    /// constructor's default, [`SpriteReads::to_object`]) and `on_target_moved` (`+0x61`), as
    /// the list adds it ([`MoveToAction::added`]). An object in a state above [`LAST_TAKEN`] is
    /// not its target (`FUN_140a7a230`: the id −1, the point stays) and is far
    /// ([`NO_TARGET_GAP`]).
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_object(
        &self,
        to: WalkTo,
        range: f32,
        given: Option<i16>,
        on_target_moved: TargetMoved,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let refused = to.state.is_some_and(|s| LAST_TAKEN < s);
        let (target, gap) = if refused {
            (-1, NO_TARGET_GAP)
        } else {
            (to.id, to.gap)
        };
        let walk = MoveToAction {
            state: MoveTo::Start,
            point: to.at,
            target,
            range,
            sprite: self.sprites.to_object(given)?,
            quiet: true,
            on_target_moved,
            wait: MoveWait::NEW,
        };
        Ok(walk.added(self.state, gap, self.speed))
    }

    /// `FUN_140aa0df0` without a target: no target, its zeroed point ([`NO_TARGET_POINT`]), far
    /// ([`NO_TARGET_GAP`]); else as [`MoveToReads::to_object`].
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_nothing(
        &self,
        range: f32,
        given: Option<i16>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let walk = MoveToAction {
            state: MoveTo::Start,
            point: NO_TARGET_POINT,
            target: -1,
            range,
            sprite: self.sprites.to_object(given)?,
            quiet: true,
            on_target_moved: TargetMoved::Follows,
            wait: MoveWait::NEW,
        };
        Ok(walk.added(self.state, NO_TARGET_GAP, self.speed))
    }

    /// A quiet `MoveTo` to `point` within `range` (`FUN_140aa0d30`, then `FUN_140a7a170` with 1)
    /// with the sprite `given` (else the constructor's default, [`SpriteReads::to_point`]), as
    /// the list adds it: far ([`NO_TARGET_GAP`]).
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_point(
        &self,
        point: [f32; 3],
        range: f32,
        given: Option<i16>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        let walk = MoveToAction::sub_to_point(point, range, self.sprites.to_point(given)?);
        Ok(walk.added(self.state, NO_TARGET_GAP, self.speed))
    }
}

impl Unit {
    /// What a new `MoveTo` reads of this unit ([`MoveToReads::of`] with its master), `player`'s, whose
    /// masters are `every`, with the sprites' table `graphics`.
    ///
    /// # Errors
    ///
    /// A unit whose master the table lacks.
    pub(crate) fn move_to_reads<'g>(
        &self,
        every: &[Masters],
        player: usize,
        graphics: &'g BTreeMap<i16, Graphic>,
    ) -> Result<MoveToReads<'g>, GenieError> {
        Ok(MoveToReads::of(self, self.master(every, player)?, graphics))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::move_to::MoveDefaults;

    /// The sub-action `MoveTo`s (`FUN_140aa0df0`, `FUN_140aa0d30`, then `FUN_140a977e0`): one to an
    /// object takes its maker's sprite, else the object default; one to a point the point default;
    /// each is quiet and new. Within 0.01 of its target it arrived, else it starts (13 without
    /// speed); a target above state 6 is refused (far, the point stays its place); one without a
    /// target walks to (0, 0, 0); a unit in state 3 or later adds none; a sprite the table lacks
    /// is an error.
    #[test]
    fn a_sub_move_to_takes_its_sprite_target_and_start() {
        let graphics = crate::construction::build::tests::graphics();
        let move_reads = MoveToReads {
            sprites: SpriteReads {
                defaults: MoveDefaults {
                    point: Some(20),
                    object: Some(21),
                },
                graphics: &graphics,
            },
            state: 2,
            speed: 1.0,
            at: [10.0, 10.0, 0.0],
            ground: [0.2, 0.2],
        };
        let house = WalkTo {
            id: 10028,
            at: [10.5, 10.5, 0.0],
            gap: 5.0,
            state: Some(0),
        };
        let to = |w: &MoveToReads<'_>, to, given| {
            w.to_object(to, 0.1, given, TargetMoved::Follows)
                .unwrap()
                .unwrap()
        };
        let m = to(&move_reads, house, None);
        assert_eq!(
            (m.state, m.target, m.point, m.range),
            (MoveTo::Start, 10028, [10.5, 10.5, 0.0], 0.1)
        );
        assert_eq!(
            (m.sprite.map(|g| g.id), m.quiet, m.wait),
            (Some(21), true, MoveWait::NEW)
        );
        assert_eq!(
            to(&move_reads, house, Some(3)).sprite.map(|g| g.id),
            Some(3)
        );
        let near = WalkTo { gap: 0.01, ..house };
        assert_eq!(to(&move_reads, near, None).state, MoveTo::Arrived);
        let gone = WalkTo {
            gap: 0.0,
            state: Some(7),
            ..house
        };
        let refused = to(&move_reads, gone, None);
        assert_eq!(
            (refused.state, refused.target, refused.point),
            (MoveTo::Start, -1, house.at)
        );
        let stuck = MoveToReads {
            speed: 0.0,
            ..move_reads
        };
        assert_eq!(to(&stuck, house, None).state, MoveTo::Failed);
        let nothing = move_reads.to_nothing(0.1, None).unwrap().unwrap();
        assert_eq!(
            (nothing.state, nothing.target, nothing.point),
            (MoveTo::Start, -1, [0.0; 3])
        );
        let point = move_reads
            .to_point([4.0, 5.0, 0.0], 0.0, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                point.state,
                point.target,
                point.range,
                point.sprite.map(|g| g.id)
            ),
            (MoveTo::Start, -1, 0.0, Some(20))
        );
        let dying = MoveToReads {
            state: 3,
            ..move_reads
        };
        assert_eq!(
            dying.to_object(house, 0.1, None, TargetMoved::Follows),
            Ok(None)
        );
        assert_eq!(dying.to_point([4.0, 5.0, 0.0], 0.0, None), Ok(None));
        assert!(move_reads.to_point([4.0, 5.0, 0.0], 0.0, Some(99)).is_err());
    }
}
