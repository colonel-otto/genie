//! Captures and takeovers: an animal or artifact a player takes over, its stop, the unit AI's
//! ownership change, a takeover by sight, a unit's ownership change, and the scan for more
//! livestock after a capture.

use crate::GenieError;
use crate::math::rng::Stream;
use crate::objects::unit::Unit;
use crate::ownership::change::change_hands;
use crate::world::context::Context;
use crate::world::others::OthersRef;

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
    pub(crate) fn capture(
        &mut self,
        c: &mut Context<'_>,
        new: usize,
    ) -> Result<(bool, Vec<i32>), GenieError> {
        let class = self.placed_class(c)?;
        self.capture_stop(c, class)?;
        let changed = self.change_owner(c, new)?;
        // the scan reads the owner after the change: the old one when `new` lacks the master
        let mut taken = Vec::new();
        if class == LIVESTOCK && (if changed { new } else { c.player }) != 0 {
            scan_livestock(c, self.position, new, &mut taken)?;
        }
        Ok((changed, taken))
    }

    /// The master class the world files the captured object under.
    pub(crate) fn placed_class(&self, c: &Context<'_>) -> Result<i16, GenieError> {
        c.w.spatial
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
    /// [`Artifact::stop`]: crate::ownership::artifact::Artifact::stop
    pub(crate) fn capture_stop(
        &mut self,
        c: &mut Context<'_>,
        class: i16,
    ) -> Result<(), GenieError> {
        let by = self.releaser(c);
        if let Some(artifact) = self.actions.artifact_mut() {
            let (gone, released) = artifact.stop();
            if gone.stops {
                self.body
                    .mover_mut()
                    .ok_or(GenieError::unsupported(
                        "a walking animal without movement state",
                    ))?
                    .stop()?;
            }
            for t in [gone.released, released].into_iter().flatten() {
                crate::world::lets::release(c, t, by)?;
            }
            self.show_standing(
                c.w.masters.get(c.player),
                &c.w.graphics,
                c.w.streams.get(Stream::Sprite),
            )?;
        } else {
            // any other top action's slot 0x38 is the base's false for what an animal or an
            // artifact holds (a `MoveTo` of its own), and the list goes
            let (mut releases, mut leaves) = (Vec::new(), Vec::new());
            self.tear_down_actions(&mut releases, &mut leaves, by)?;
            crate::world::lets::torn_down(c, (leaves, releases))?;
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
    ///
    /// [`Step::Captured`]: crate::objects::unit::Step::Captured
    pub(crate) fn ai_change_owner(
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
    pub(crate) fn take_by_sight(
        &mut self,
        c: &mut Context<'_>,
        new: usize,
    ) -> Result<(), GenieError> {
        let master = self.own_master.as_deref().or_else(|| {
            c.w.masters
                .get(c.player)
                .and_then(|ms| ms.get(self.life.master_id))
        });
        if master.is_some_and(|m| m.class == SIEGE_CLASS) && !self.garrison.is_empty() {
            return Err(HANDS_LEFT);
        }
        let _ = self.change_owner(c, new)?;
        self.capture_flag = 0;
        if let Some(s) = c.w.spatial.sight.as_mut().and_then(|v| v.seer_mut(self.id)) {
            s.capture = 0;
        }
        Ok(())
    }

    /// The ownership change (slot 0x70, [`change_hands`]) of the unit, from its owner to `new`:
    /// false when `new` has not its master. A building's (`FUN_1411cd860`) is the caller's after
    /// the update, whole ([`Captured::building`], [`crate::world::World::building_changes_hands`]),
    /// whether or not `new` has its master.
    pub(crate) fn change_owner(
        &mut self,
        c: &mut Context<'_>,
        new: usize,
    ) -> Result<bool, GenieError> {
        let master = self.own_master.as_deref().or_else(|| {
            c.w.masters
                .get(c.player)
                .and_then(|ms| ms.get(self.life.master_id))
        });
        if master.is_some_and(|m| m.kind == crate::objects::masters::master_type::BUILDING) {
            c.building_handover = true;
            return Ok(c
                .w
                .masters
                .get(new)
                .is_some_and(|m| m.get(self.life.master_id).is_some()));
        }
        change_hands(
            self,
            (c.player, new),
            &c.w.masters,
            &mut c.w.spatial,
            c.world_time,
        )
    }
}

/// The livestock class (`-0xe72` raw).
pub(crate) const LIVESTOCK: i16 = 58;

/// The siege class (`-0xe9f` raw), whose ownership change drops the units it carries
/// (`FUN_14120f800`).
pub(crate) const SIEGE_CLASS: i16 = 13;

/// The ownership changes the slice does not model: a building's (its slot 0x70,
/// `FUN_1411cd860`: its annexes', its production queue's and researches', its garrison's, its
/// object and technology on capture, then the combat object's), which a takeover by sight and
/// the artifact's ask for one reach, and a siege weapon's with units inside (`FUN_14120f800`'s
/// class 13 branch: `FUN_141192d10`, each unit's leave, slot 0x1a8, with the siege weapon's slot
/// 0x1b0, and the rest destroyed, slot 0xd0).
pub(crate) const HANDS_LEFT: GenieError = GenieError::unsupported(
    "an ownership change of a building or of a siege weapon with units inside",
);

/// The trade action's type (slot 0x438), whose load keeps a captured unit's unit AI going
/// (`FUN_14108c050`).
pub(crate) const TRADE_ACTION: i32 = 0x6f;

/// The unit AI's action that a capture's stop leaves alone (`FUN_14108c050`).
pub(crate) const CAPTURE_KEEPS_ACTION: i32 = 0x2c2;

/// `FUN_140aab570`'s scan after a livestock's capture: Gaia's livestock in state 2 whose tile
/// the new owner sees (`FUN_141663500`), within 3 tiles of `at` (x outer, y inner); each is taken
/// in turn, with its own scan. Two on one tile would need the tile's object list order.
pub(crate) fn scan_livestock(
    c: &mut Context<'_>,
    at: [f32; 3],
    new: usize,
    taken: &mut Vec<i32>,
) -> Result<(), GenieError> {
    let (tx, ty) = (at[0] as i32, at[1] as i32);
    let [w, h] = c.w.spatial.map;
    for x in (tx - 3).max(0)..=(tx + 3).min(w - 1) {
        for y in (ty - 3).max(0)..=(ty + 3).min(h - 1) {
            if !c.w.spatial.sees_tile(new, x, y) {
                continue;
            }
            let on: Vec<i32> = OthersRef::of(&c.w.players, c.player)
                .own_iter()
                .filter(|u| {
                    u.life.state == 2
                        && u.position[0] as i32 == x
                        && u.position[1] as i32 == y
                        && !taken.contains(&u.id)
                        && c.w
                            .spatial
                            .get(u.id)
                            .is_some_and(|p| p.class == LIVESTOCK && p.player == 0)
                })
                .map(|u| u.id)
                .collect();
            match on[..] {
                [] => {}
                [id] => {
                    // out of its slot while its stop reaches the view, as the updating object
                    let mut unit =
                        c.w.take_out_own(c.player, id)
                            .expect("the scan's own object");
                    let done = (|| {
                        unit.capture_stop(c, LIVESTOCK)?;
                        unit.change_owner(c, new)
                    })();
                    let position = unit.position;
                    let _ = c.w.put_back_own(c.player, unit)?;
                    // without the master the animal stays Gaia's, which scans nothing
                    if done? {
                        taken.push(id);
                        scan_livestock(c, position, new, taken)?;
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

/// What a takeover in an object's update did ([`Step::Captured`]): player `new` took the object
/// (`main`; without the master it stays) and then `also`, other objects of its player, in order.
/// Those taken move to the end of the new owner's list; their storage and class counters move
/// with them.
///
/// [`Step::Captured`]: crate::objects::unit::Step::Captured
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Captured {
    pub(crate) new: usize,
    pub(crate) main: bool,
    pub(crate) also: Vec<i32>,
    /// The object is a building: its slot 0x70 (`FUN_1411cd860`) is the caller's, whole
    /// ([`crate::world::World::building_changes_hands`]); `main` says whether the new owner has
    /// its master.
    pub(crate) building: bool,
}
