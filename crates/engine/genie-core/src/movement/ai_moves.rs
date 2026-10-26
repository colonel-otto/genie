//! A unit's moves its unit AI asks: to a point (slot 0x340, the head action's slot 0x40 may take
//! the point) and to an object (slot 0x348), a move that gave up, and the standing sprite of a
//! `MoveTo` that ends at its start.

use crate::GenieError;
use crate::construction::build::BUILD;
use crate::math::rng::{Stream, Well512};
use crate::movement::move_to::{MoveTo, MoveToAction};
use crate::objects::masters::Masters;
use crate::objects::release::Releaser;
use crate::objects::sprite::Graphic;
use crate::objects::unit::Unit;
use crate::ownership::capture::LIVESTOCK;
use crate::unit_ai::AiMove;
use crate::world::context::Context;

/// An unpacked trebuchet's class (`-0xe76` raw), whose slot 0x340 packs instead of moving.
const UNPACKED_TREBUCHET: i16 = 54;

impl Unit {
    /// The unit's slot 0x340 on its unit AI's move ([`Unit::ai_move_with`]), what it lets go and
    /// the standing sprite of a `MoveTo` that fails at its start into the context `c`.
    pub(crate) fn ai_move(
        &mut self,
        m: AiMove,
        class: i16,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        let by = self.releaser(c);
        let sprite_rng = c.streams.get(Stream::Sprite);
        let sails = self.ai_move_with(
            m,
            (class, c.player),
            (&mut c.releases, &mut c.builder_leaves, by),
            (c.masters, c.graphics, sprite_rng),
        )?;
        if sails {
            let sprite_rng = c.streams.get(Stream::Sprite);
            self.transport_sail(
                m.point,
                (c.spatial, c.zones, c.pathing),
                (c.masters, c.graphics, sprite_rng),
            )?;
        }
        Ok(())
    }

    /// The unit's slot 0x340 (`FUN_141216c50`) on a move to `m.point` its unit AI gives it (also
    /// its move off a footprint, [`crate::world::World::move_aside`]), the unit of `class` of
    /// player `player`. An unpacked trebuchet (class 54) packs instead (its Pack action,
    /// `FUN_140ad4ff0` through slot 0x808), which is not modelled; a packed one (class 51) whose
    /// head action packs or unpacks (types `0x7c`, `0x7d`) does nothing, and the slice holds no such
    /// action, so it moves as any other: `FUN_1410866e0`. Without `m.force` a head `MoveTo` whose
    /// point (`+0x38`..`+0x40`, [`MoveToAction::point`]) is `m.point` stands: the move is done.
    /// Then the head action's slot 0x40 may take the point ([`Unit::head_takes_point`]; the
    /// target it lets go into `releases`). Otherwise a `MoveTo` to the point within `m.range`
    /// (`FUN_140aa0d30`, the master's walking sprite with `m.walk`, else its running one; the
    /// sprite is not held) is made, the action list is cleared (`FUN_140a98620`,
    /// [`Unit::clear_own_actions`]: the teardowns let their targets go into `releases` and a
    /// counted builder's foundation into `builder_leaves`, by `by`) and the `MoveTo` installed and
    /// started (slot 0x808, `FUN_14106e990`: its slot 0x70 sees an empty list; `FUN_140a977e0`
    /// starts an action of a unit below state 3, which the unit AI's moves are, and
    /// [`MoveToAction::to_point`]); one that ends at its start shows the standing sprite of the
    /// unit's master in `look` ([`Unit::shows_standing_sprite`]). True when a Transport on top
    /// takes the point, which the caller sails there ([`Unit::transport_sail`]).
    pub(crate) fn ai_move_with(
        &mut self,
        m: AiMove,
        (class, player): (i16, usize),
        (releases, builder_leaves, by): (&mut Vec<(i32, Releaser)>, &mut Vec<i32>, Releaser),
        (masters, graphics, sprite_rng): (
            Option<&Masters>,
            &std::collections::BTreeMap<i16, Graphic>,
            &mut Well512,
        ),
    ) -> Result<bool, GenieError> {
        if class == UNPACKED_TREBUCHET {
            return Err(GenieError::unsupported(
                "an AI move of an unpacked trebuchet (class 54: its Pack action, FUN_140ad4ff0)",
            ));
        }
        if !m.force
            && self.top_action_kind() == Some(crate::movement::move_to::MOVE_TO)
            && self
                .move_to
                .and_then(|mt| mt.point(self.mover.as_ref()))
                .is_some_and(|p| p == m.point)
        {
            return Ok(false);
        }
        match self.head_takes_point(m.point, player)? {
            PointMove::Taken(released) => {
                if let Some(t) = released {
                    releases.push((t, by));
                }
                return Ok(false);
            }
            PointMove::Repairs(released, left) => {
                builder_leaves.extend(left);
                if let Some(t) = released {
                    releases.push((t, by));
                }
                return Ok(false);
            }
            PointMove::Sails => return Ok(true),
            PointMove::Declined => {}
        }
        self.clear_own_actions(releases, builder_leaves, by)?;
        let move_to = MoveToAction::to_point(m.point, m.range, !m.walk, self.mover.as_mut());
        if move_to.shows_standing() {
            self.shows_standing_sprite(masters, graphics, sprite_rng)?;
        }
        self.move_to = Some(move_to);
        Ok(false)
    }

    /// The head action's slot 0x40 on a move to `point` without a target object (its second
    /// argument 0), the unit of player `player`. An empty list takes nothing
    /// (`FUN_140a98730`), nor does the base slot (`FUN_140a78490`, which answers 0) of a `MoveTo`,
    /// an Explore, a Build (`0x140aade60`), an Attack, an Enter, a Fly, a missile, a Make or a
    /// wait for a foundation. The Artifact's (`FUN_140aaca30`) takes it
    /// ([`Artifact::take_point`]; a `MoveTo` it walked goes and stops the unit); the Hunt's
    /// (`FUN_140ac52c0`, [`HuntAction::take_move`]) unless its task has a work range; the
    /// Gather's (`0x140a8b740`, [`crate::gathering::gather::GatherAction::take_move`]) always. The Hunt and
    /// the Gather let their target go (slot 0x68 with none), which [`PointMove::Taken`] names
    /// for the caller to release; their second target stays.
    ///
    /// # Errors
    ///
    /// An action the slice does not hold, and what the Hunt's and the Gather's walks do not
    /// model.
    ///
    /// [`Artifact::take_point`]: crate::ownership::artifact::Artifact::take_point
    /// [`HuntAction::take_move`]: crate::gathering::hunt::HuntAction::take_move
    pub(crate) fn head_takes_point(
        &mut self,
        point: [f32; 3],
        player: usize,
    ) -> Result<PointMove, GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "slot 0x40 of an action the slice does not hold",
            ));
        }
        match self.top_action_kind() {
            None
            | Some(
                crate::movement::move_to::MOVE_TO
                | crate::movement::explore::EXPLORE
                | BUILD
                | crate::combat::attack::ATTACK
                | crate::garrison::ENTER
                | crate::movement::fly::FLY
                | crate::combat::missile::MISSILE
                | crate::gathering::make::MAKE
                | crate::construction::wait_solidify::WAIT_SOLIDIFY,
            ) => Ok(PointMove::Declined),
            Some(crate::ownership::artifact::ARTIFACT_TYPE) => {
                let gone = self
                    .artifact
                    .as_mut()
                    .map(|a| a.take_point(point))
                    .unwrap_or_default();
                if gone.stops {
                    self.mover
                        .as_mut()
                        .ok_or(crate::world::WALKER_UNMOVING)?
                        .stop()?;
                }
                // the Enter it held lets its building or transport go
                Ok(PointMove::Taken(gone.released))
            }
            Some(crate::gathering::hunt::HUNT) => {
                let (held, my_pasture) = (self.life.amount, self.my_pasture);
                let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
                    return Err(GenieError::unsupported(
                        "a hunter without its hunt or movement state",
                    ));
                };
                let old = hunt.target;
                if !hunt.take_move(None, point, player, held, false, my_pasture, 0.0, mover)? {
                    return Ok(PointMove::Declined);
                }
                Ok(PointMove::Taken(Some(old).filter(|&t| t != -1)))
            }
            Some(crate::gathering::gather::GATHER) => {
                let held = self.life.amount;
                let (Some(gather), Some(mover)) = (self.gather.as_mut(), self.mover.as_mut())
                else {
                    return Err(GenieError::unsupported(
                        "a gatherer without its gather or movement state",
                    ));
                };
                let old = gather.target;
                gather.take_move(None, point, player, held, mover)?;
                Ok(PointMove::Taken(Some(old).filter(|&t| t != -1)))
            }
            // the Transport's (0x140aa3e40): the caller sails it there
            Some(crate::garrison::transport::TRANSPORT) => Ok(PointMove::Sails),
            // the Repair's (FUN_140ad9700): its point, slot 0x68 with none, set_state(11)
            Some(crate::construction::repair::REPAIR) => {
                let repair = self.repair.as_mut().expect("the unit's top action");
                let (old, counted) = repair.take_point(point, self.mover.as_mut())?;
                Ok(PointMove::Repairs(
                    Some(old).filter(|&t| t != -1),
                    counted.then_some(old),
                ))
            }
            Some(other) => {
                unreachable!("a top action of type {other}, which top_action never holds")
            }
        }
    }

    /// The unit's slot 0x348 (`FUN_141086440`), forced, on its unit AI's move to an object (in
    /// `state`): a `MoveTo` to the target within `range` (none: slot 0x268's,
    /// [`crate::movement::move_to::object_range`]), showing the master's running sprite (the
    /// walking one without it), takes the target it holds (`FUN_140aa0df0`, quiet 0: slot 0x178,
    /// [`crate::movement::move_to::holds_object`]), then replaces its action list (`FUN_140a98620`,
    /// [`Unit::clear_own_actions`]: the old actions' teardowns let their targets go) and starts
    /// ([`MoveToAction::to_object`]).
    pub(crate) fn move_to_object(
        &mut self,
        (target, state): (i32, u8),
        range: f32,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "an AI move to an object of a unit with an action",
            ));
        }
        if self.mover.is_none() {
            return Err(GenieError::unsupported(
                "an AI move to an object of a unit that does not move",
            ));
        }
        let livestock = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .is_some_and(|m| m.class == LIVESTOCK);
        let master = c
            .spatial
            .get_known(target)?
            .ok_or(crate::movement::move_to::TARGET_LACKING)?
            .master;
        if crate::movement::move_to::holds_object(livestock, master, state) {
            c.takes.push((target, self.releaser(c)));
        }
        let by = self.releaser(c);
        self.clear_own_actions(&mut c.releases, &mut c.builder_leaves, by)?;
        let own = c.masters.and_then(|m| m.get(self.life.master_id));
        let range = crate::movement::move_to::object_range(range, own);
        let mover = self.mover.as_mut().expect("tested above");
        let move_to = MoveToAction::to_object(
            c.spatial,
            self.position,
            mover,
            (target, state),
            range,
            (true, livestock),
        )?;
        if move_to.shows_standing() {
            self.shows_standing_sprite(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
        }
        self.move_to = Some(move_to);
        Ok(())
    }

    /// `LAB_140aa320c`, the motion step gave up on the unit's own `MoveTo`: a human's villager
    /// (`waits`) waits once (`set_state(2)`, 1 s or 2 s by its `stance`, which shows nothing);
    /// any other unit's `MoveTo` fails (`set_state(13)`), which its unit AI's check between turns may
    /// interrupt with its next order the same update. The failure's `set_state` (`0x140aa29e0`,
    /// an own `MoveTo` is never quiet) shows the standing sprite at its speed and clears
    /// `+0x7b8`.
    ///
    /// # Errors
    ///
    /// A unit with a sprite whose master's standing sprite the slice does not hold.
    pub(crate) fn own_move_gave_up(
        &mut self,
        waits: bool,
        stance: u8,
        (masters, graphics, sprite_rng): (
            Option<&Masters>,
            &std::collections::BTreeMap<i16, Graphic>,
            &mut Well512,
        ),
    ) -> Result<(), GenieError> {
        let Some(m) = self.move_to.as_mut() else {
            return Ok(());
        };
        m.state = m.wait.give_up(waits, stance);
        if m.state == MoveTo::Failed {
            if let Some(mover) = self.mover.as_mut() {
                mover.shows_standing();
            }
            self.shows_standing_sprite(masters, graphics, sprite_rng)?;
        }
        Ok(())
    }

    /// A `MoveTo`'s `set_state` 1, not quiet, on the unit (`0x140aa29e0`): the master's standing
    /// sprite where the slice holds the unit's sprite (slot 0x68 with slot 0x798's, a draw of the
    /// Sprite stream), and its animation speed (`+0x7b8`) cleared where known.
    pub(crate) fn shows_standing_sprite(
        &mut self,
        masters: Option<&Masters>,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite_rng: &mut Well512,
    ) -> Result<(), GenieError> {
        if self.sprite.is_some() {
            let standing = masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.standing_sprite)
                .and_then(|id| graphics.get(&id).copied())
                .ok_or(GenieError::unsupported(
                    "an arrived walker without its standing sprite",
                ))?;
            crate::combat::attack::set_sprite(&mut self.sprite, standing, sprite_rng);
        }
        if let Some(a) = self.anim_speed.as_mut() {
            *a = 0.0;
        }
        Ok(())
    }
}

/// What the head action's slot 0x40 did with a move to a point ([`Unit::head_takes_point`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointMove {
    /// It answered 0 (an empty list, the base slot, a Hunt whose task has a work range): the
    /// caller's `MoveTo` replaces the actions.
    Declined,
    /// It answered 1: the action walks there itself. The target it let go (slot 0x68 with none),
    /// if any, which the caller releases.
    Taken(Option<i32>),
    /// A Repair answered 1 (`FUN_140ad9700`): it walks to the point; the target it let go, and
    /// the target whose repairers a counted repairer left ([`crate::world::World::leave_builders`]).
    Repairs(Option<i32>, Option<i32>),
    /// The Transport answers 1 (`0x140aa3e40`): it sails to the point, which the caller runs
    /// with the map ([`Unit::transport_sail`]).
    Sails,
}
