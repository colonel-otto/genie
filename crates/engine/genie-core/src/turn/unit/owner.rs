//! Captures and ownership changes, the unit AI's moves and a move to an object.

use crate::build::BUILD;
use crate::masters::Masters;
use crate::math::rng::{Stream, Well512};
use crate::move_to::{MoveTo, MoveToAction};
use crate::sprite::Graphic;
use crate::turn::unit::{UNPACKED_TREBUCHET, Unit};
use crate::turn::view::Context;
use crate::turn::world::{Releaser, change_hands};
use crate::unit_ai::{AiMove, GenieError};

impl Unit {
    /// `FUN_140aab570`: player `new` takes the animal over. Its stop (slot 0x148,
    /// `FUN_14108c050` -> `FUN_14106fa60`): its Artifact action's slot 0x38 (the unit AI of a
    /// class other than livestock would stop too, which is not modelled), then the ownership
    /// change (slot 0x70, [`Unit::change_owner`]); a livestock whose owner is then not Gaia has
    /// its new owner take Gaia's livestock in state 2 on the tiles it sees within 3 of the
    /// animal's tile, x then y, each the same way. Returns whether the animal changed hands and
    /// the others taken; the lists' moves, the storage and the class counters are the caller's.
    /// The new owner's computer AI part (`FUN_140aab130`), the local player's sound and
    /// statistics are not held.
    pub(in crate::turn) fn capture(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        new: usize,
    ) -> Result<(bool, Vec<i32>), GenieError> {
        let class = self.placed_class(c)?;
        self.capture_stop(c, class)?;
        let changed = self.change_owner(c, new)?;
        // the scan reads the owner after the change: the old one when `new` lacks the master
        let mut taken = Vec::new();
        if class == LIVESTOCK && (if changed { new } else { c.player }) != 0 {
            scan_livestock(c, others, self.position, new, &mut taken)?;
        }
        Ok((changed, taken))
    }

    /// The master class the world files the captured object under.
    pub(in crate::turn) fn placed_class(&self, c: &Context<'_>) -> Result<i16, GenieError> {
        c.spatial
            .get(self.id)
            .map(|p| p.class)
            .ok_or(GenieError::unsupported(
                "a captured object the world does not hold",
            ))
    }

    /// The stop (slot 0x148, `FUN_14108c050`) of an animal or artifact of master class `class`
    /// from inside its update: the action object's stop (`FUN_14106fa60`: `+0x88a` 0, the top
    /// action's slot 0x38: the Artifact's, [`Artifact::stop`], keeps the action list: a walk's
    /// `MoveTo` goes and stops the unit, the target is let go, and the unit shows its standing
    /// sprite; without an Artifact the list goes, an animal or an artifact holding no Hunt,
    /// Gather, Build or Attack, whose slot 0x38 would keep it); then a unit AI of any class but
    /// livestock, not on action 0x2c2, stops (slot 0xb0 with 0: the order ends, the unit's
    /// stopAction) and forgets its targets (`FUN_140c4dcb0`), unless the unit's top action
    /// (slot 0x438) is a trade (0x6f) with a load of a kind (COMISS 0, amount; JNC: an amount
    /// that is not a number keeps it).
    ///
    /// [`Artifact::stop`]: crate::artifact::Artifact::stop
    pub(in crate::turn) fn capture_stop(
        &mut self,
        c: &mut Context<'_>,
        class: i16,
    ) -> Result<(), GenieError> {
        let by = self.releaser(c);
        if let Some(artifact) = self.artifact.as_mut() {
            let (gone, released) = artifact.stop();
            if gone.stops {
                self.mover
                    .as_mut()
                    .ok_or(GenieError::unsupported(
                        "a walking animal without movement state",
                    ))?
                    .stop()?;
            }
            for t in [gone.released, released].into_iter().flatten() {
                c.releases.push((t, by));
            }
            self.show_standing(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
        } else {
            // any other top action's slot 0x38 is the base's false for what an animal or an
            // artifact holds (a `MoveTo` of its own), and the list goes
            self.tear_down_actions(&mut c.releases, &mut c.builder_leaves, by)?;
        }
        let top = self.top_action_kind().map_or(-1, i32::from);
        let trading = top == TRADE_ACTION
            && crate::math::fmath::below_or_unordered(0.0, self.life.amount)
            && self.life.attribute_type != -1;
        if class != LIVESTOCK
            && !trading
            && let Some(ai) = self.ai.as_mut()
            && ai.current_action != CAPTURE_KEEPS_ACTION
        {
            let mut fx = Vec::new();
            let place = ([self.position[0], self.position[1]], class);
            ai.stop_with(false, Some(place), &mut fx)?;
            ai.forget_targets();
            self.apply(fx, Some(class), c)?;
        }
        Ok(())
    }

    /// The unit's slot 0x70 as its unit AI asks it ([`Effect::ChangeOwner`], the artifact's
    /// processIdle `FUN_140c1e410`): [`Unit::change_owner`]; once it changed hands the caller
    /// moves it to the new owner's list after the update ([`Step::Captured`]), and a building's
    /// whole change is the caller's even without the new owner's master.
    ///
    /// [`Effect::ChangeOwner`]: crate::unit_ai::Effect::ChangeOwner
    pub(in crate::turn) fn ai_change_owner(
        &mut self,
        new: usize,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        let changed = self.change_owner(c, new)?;
        if changed || c.building_handover {
            c.taken_over = Some((new, changed));
        }
        Ok(())
    }

    /// `RGE_Combat_Object::update`'s takeover by sight of the unit by player `new`, which has its
    /// master: slot 0x70 ([`Unit::change_owner`]; a building's is the caller's after the update),
    /// then `+0x924` goes 0. The local player's sound and its capture notice (`FUN_1414c3380`)
    /// are not simulation. Not modelled: a siege weapon's (class 13) drop of the units it carries
    /// (`FUN_14120f800` with `FUN_141192d10`).
    pub(in crate::turn) fn take_by_sight(
        &mut self,
        c: &mut Context<'_>,
        new: usize,
    ) -> Result<(), GenieError> {
        let master = self
            .own_master
            .as_deref()
            .or_else(|| c.masters.and_then(|ms| ms.get(self.life.master_id)));
        if master.is_some_and(|m| m.class == SIEGE_CLASS) && !self.garrison.is_empty() {
            return Err(HANDS_LEFT);
        }
        let _ = self.change_owner(c, new)?;
        self.capture_flag = 0;
        if let Some(s) = c.spatial.sight.as_mut().and_then(|v| v.seer_mut(self.id)) {
            s.capture = 0;
        }
        Ok(())
    }

    /// The ownership change (slot 0x70, [`change_hands`]) of the unit, from its owner to `new`:
    /// false when `new` has not its master. A building's (`FUN_1411cd860`) is the caller's after
    /// the update, whole ([`Captured::building`], [`crate::turn::Turns::building_changes_hands`]),
    /// whether or not `new` has its master.
    pub(in crate::turn) fn change_owner(
        &mut self,
        c: &mut Context<'_>,
        new: usize,
    ) -> Result<bool, GenieError> {
        let master = self
            .own_master
            .as_deref()
            .or_else(|| c.masters.and_then(|ms| ms.get(self.life.master_id)));
        if master.is_some_and(|m| m.kind == crate::masters::master_type::BUILDING) {
            c.building_handover = true;
            return Ok(c
                .every_masters
                .get(new)
                .is_some_and(|m| m.get(self.life.master_id).is_some()));
        }
        change_hands(
            self,
            (c.player, new),
            c.every_masters,
            c.spatial,
            c.world_time,
        )
    }

    /// The unit's slot 0x340 on its unit AI's move ([`Unit::ai_move_with`]), what it lets go and
    /// the standing sprite of a `MoveTo` that fails at its start into the context `c`.
    pub(in crate::turn) fn ai_move(
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
    /// its move off a footprint, [`crate::turn::Turns::move_aside`]), the unit of `class` of
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
            && self.top_action_kind() == Some(crate::move_to::MOVE_TO)
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
    /// [`Artifact::take_point`]: crate::artifact::Artifact::take_point
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
                crate::move_to::MOVE_TO
                | crate::explore::EXPLORE
                | BUILD
                | crate::combat::attack::ATTACK
                | crate::garrison::ENTER
                | crate::fly::FLY
                | crate::combat::missile::MISSILE
                | crate::gathering::make::MAKE
                | crate::wait_solidify::WAIT_SOLIDIFY,
            ) => Ok(PointMove::Declined),
            Some(crate::artifact::ARTIFACT_TYPE) => {
                let gone = self
                    .artifact
                    .as_mut()
                    .map(|a| a.take_point(point))
                    .unwrap_or_default();
                if gone.stops {
                    self.mover
                        .as_mut()
                        .ok_or(crate::turn::WALKER_UNMOVING)?
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
            Some(crate::transport::TRANSPORT) => Ok(PointMove::Sails),
            // the Repair's (FUN_140ad9700): its point, slot 0x68 with none, set_state(11)
            Some(crate::repair::REPAIR) => {
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
    /// [`crate::move_to::object_range`]), showing the master's running sprite (the
    /// walking one without it), takes the target it holds (`FUN_140aa0df0`, quiet 0: slot 0x178,
    /// [`crate::move_to::holds_object`]), then replaces its action list (`FUN_140a98620`,
    /// [`Unit::clear_own_actions`]: the old actions' teardowns let their targets go) and starts
    /// ([`MoveToAction::to_object`]).
    pub(in crate::turn) fn move_to_object(
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
            .ok_or(crate::move_to::TARGET_LACKING)?
            .master;
        if crate::move_to::holds_object(livestock, master, state) {
            c.takes.push((target, self.releaser(c)));
        }
        let by = self.releaser(c);
        self.clear_own_actions(&mut c.releases, &mut c.builder_leaves, by)?;
        let own = c.masters.and_then(|m| m.get(self.life.master_id));
        let range = crate::move_to::object_range(range, own);
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
    /// the target whose repairers a counted repairer left ([`crate::turn::Turns::leave_builders`]).
    Repairs(Option<i32>, Option<i32>),
    /// The Transport answers 1 (`0x140aa3e40`): it sails to the point, which the caller runs
    /// with the map ([`Unit::transport_sail`]).
    Sails,
}

/// The population's resource, which a cost check skips (`FUN_141618db0` with its fifth argument
/// 1).
pub(in crate::turn) const POPULATION: i16 = 4;

/// The livestock class (`-0xe72` raw).
pub(in crate::turn) const LIVESTOCK: i16 = 58;

/// The siege class (`-0xe9f` raw), whose ownership change drops the units it carries
/// (`FUN_14120f800`).
pub(in crate::turn) const SIEGE_CLASS: i16 = 13;

/// The ownership changes the slice does not model: a building's (its slot 0x70,
/// `FUN_1411cd860`: its annexes', its production queue's and researches', its garrison's, its
/// object and technology on capture, then the combat object's), which a takeover by sight and
/// the artifact's ask for one reach, and a siege weapon's with units inside (`FUN_14120f800`'s
/// class 13 branch: `FUN_141192d10`, each unit's leave, slot 0x1a8, with the siege weapon's slot
/// 0x1b0, and the rest destroyed, slot 0xd0).
pub(in crate::turn) const HANDS_LEFT: GenieError = GenieError::unsupported(
    "an ownership change of a building or of a siege weapon with units inside",
);

/// The trade action's type (slot 0x438), whose load keeps a captured unit's unit AI going
/// (`FUN_14108c050`).
pub(in crate::turn) const TRADE_ACTION: i32 = 0x6f;

/// The unit AI's action that a capture's stop leaves alone (`FUN_14108c050`).
pub(in crate::turn) const CAPTURE_KEEPS_ACTION: i32 = 0x2c2;

/// `FUN_140aab570`'s scan after a livestock's capture: Gaia's livestock in state 2 whose tile
/// the new owner sees (`FUN_141663500`), within 3 tiles of `at` (x outer, y inner); each is taken
/// in turn, with its own scan. Two on one tile would need the tile's object list order.
pub(in crate::turn) fn scan_livestock(
    c: &mut Context<'_>,
    others: &mut [Unit],
    at: [f32; 3],
    new: usize,
    taken: &mut Vec<i32>,
) -> Result<(), GenieError> {
    let (tx, ty) = (at[0] as i32, at[1] as i32);
    let [w, h] = c.spatial.map;
    for x in (tx - 3).max(0)..=(tx + 3).min(w - 1) {
        for y in (ty - 3).max(0)..=(ty + 3).min(h - 1) {
            if !c.spatial.sees_tile(new, x, y) {
                continue;
            }
            let on: Vec<usize> = others
                .iter()
                .enumerate()
                .filter(|(_, u)| {
                    u.life.state == 2
                        && u.position[0] as i32 == x
                        && u.position[1] as i32 == y
                        && !taken.contains(&u.id)
                        && c.spatial
                            .get(u.id)
                            .is_some_and(|p| p.class == LIVESTOCK && p.player == 0)
                })
                .map(|(i, _)| i)
                .collect();
            match on[..] {
                [] => {}
                [k] => {
                    let mut unit = std::mem::take(&mut others[k]);
                    let done = (|| {
                        unit.capture_stop(c, LIVESTOCK)?;
                        unit.change_owner(c, new)
                    })();
                    let (id, position) = (unit.id, unit.position);
                    others[k] = unit;
                    // without the master the animal stays Gaia's, which scans nothing
                    if done? {
                        taken.push(id);
                        scan_livestock(c, others, position, new, taken)?;
                    }
                }
                _ => {
                    return Err(GenieError::unsupported(
                        "two livestock to take on one tile (its object list's order)",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// What an object's update leaves of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::turn) enum Step {
    Stay,
    /// It leaves its list for good (`RGE_Object_List::update` deletes it).
    Leave,
    /// It moves to its player's sleeping list.
    Sleep,
    /// A player took it or other objects of its player over ([`Captured`]).
    Captured(Captured),
}

/// What a takeover in an object's update did ([`Step::Captured`]): player `new` took the object
/// (`main`; without the master it stays) and then `also`, other objects of its player, in order.
/// Those taken move to the end of the new owner's list; their storage and class counters move
/// with them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::turn) struct Captured {
    pub(in crate::turn) new: usize,
    pub(in crate::turn) main: bool,
    pub(in crate::turn) also: Vec<i32>,
    /// The object is a building: its slot 0x70 (`FUN_1411cd860`) is the caller's, whole
    /// ([`crate::turn::Turns::building_changes_hands`]); `main` says whether the new owner has
    /// its master.
    pub(in crate::turn) building: bool,
}

/// A repairer's player's attributes as its repair pays from them ([`crate::repair::Payer`]):
/// the resources (`Player::resources`) and the other attributes as the update reads them, each
/// payment kept as an add for the caller ([`Context::attribute_adds`]: a subtraction, so the
/// player's value comes out as the repair saw it).
pub(super) struct PaidAttributes {
    pub(super) values: Vec<f32>,
    pub(super) adds: Vec<(i16, f32)>,
}

impl PaidAttributes {
    pub(super) fn new(resources: [f32; 4], attributes: &[f32]) -> Self {
        let mut values = attributes.to_vec();
        for (i, r) in resources.into_iter().enumerate() {
            if let Some(v) = values.get_mut(i) {
                *v = r;
            }
        }
        Self {
            values,
            adds: Vec::new(),
        }
    }
}

impl crate::tech::Attributes for PaidAttributes {
    fn get(&self, i: i16) -> Result<f32, GenieError> {
        usize::try_from(i)
            .ok()
            .and_then(|k| self.values.get(k))
            .copied()
            .ok_or(crate::turn::NO_ATTRIBUTE)
    }

    fn set(&mut self, _: i16, _: f32) -> Result<(), GenieError> {
        Err(GenieError::unreachable(
            "a repair's payment that sets an attribute",
        ))
    }

    fn subtract(&mut self, i: i16, amount: f32) -> Result<(), GenieError> {
        let v = usize::try_from(i)
            .ok()
            .and_then(|k| self.values.get_mut(k))
            .ok_or(crate::turn::NO_ATTRIBUTE)?;
        *v -= amount;
        self.adds.push((i, -amount));
        Ok(())
    }
}
