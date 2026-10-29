//! The unit AI's part of the update: its surroundings, its side and its run.

use crate::GenieError;
use crate::math::rng::{Stream, Well512};
use crate::movement::{Mover, footprint_gap_squared};
use crate::objects::lifecycle::seconds;
use crate::objects::masters::Master;
use crate::objects::unit::object_master;
use crate::objects::unit::{Outcome, Unit};
use crate::ownership::capture::Captured;
use crate::unit_ai::Notify;
use crate::unit_ai::world_view::{OwnerView, UnitView, changed_view};
use crate::unit_ai::{AfterOrder, Surroundings, UnitAi};
use crate::world::others::OthersRef;
use crate::world::scenario_mode;
use crate::world::standard_mode;
use crate::world::step::Step;

impl Unit {
    /// The end of [`Unit::update`] after the actions (`captured`: the Artifact's capture, `leaves`:
    /// the step its life step left): `RGE_Action_Object::update`'s end (`FUN_141163080`: the
    /// action object's shape), `RGE_Combat_Object::update`'s sight mask, then the unit AI's full
    /// update on its owner's turn, else its check between turns, its calls on the unit at once
    /// ([`Unit::ai_run`]); after an order on a target the full update took, the update's rest
    /// ([`crate::unit_ai::UnitAi::after_order`]), as the game runs both inside the update (slot
    /// 0x230). Then the step the update ends with.
    pub(crate) fn ai_part(
        &mut self,
        c: &mut Step<'_>,
        (captured, leaves): (Option<Captured>, Outcome),
    ) -> Result<Outcome, GenieError> {
        self.refresh_pathing(c);
        // RGE_Combat_Object::update after the actions: the object's sight mask; a changed mask of
        // an object others take over by seeing it (`+0x924` 1) gives it to the first player that
        // sees it ([`crate::vision::visibility::sight_taker`]) when that one has its master
        let mut captured = captured;
        if self.combat
            && let Some(mask) = c.w.spatial.combat_sight(self.id)?
            && let Some(q) = crate::vision::visibility::sight_taker(
                mask,
                c.player,
                standard_mode(c.w.settings.game_mode),
                &c.w.players,
                &c.w.allow_gaia_capture,
            )
            && c.w
                .masters
                .get(q)
                .is_some_and(|m| m.get(self.life.master_id).is_some())
        {
            self.take_by_sight(c, q)?;
            captured = Some(Captured {
                new: q,
                main: true,
                also: Vec::new(),
                building: std::mem::take(&mut c.building_handover),
            });
        }
        let mut after = self.ai_run(c, captured.as_ref(), None)?;
        while let Some(tail) = after {
            after = self.ai_run(c, captured.as_ref(), Some(tail))?;
        }
        if let Some((new, main)) = c.taken_over.take() {
            captured = Some(Captured {
                new,
                main,
                also: Vec::new(),
                building: std::mem::take(&mut c.building_handover),
            });
        }
        if let Some(k) = captured {
            return Ok(Outcome::Captured(k));
        }
        Ok(leaves)
    }

    /// What the unit's AI reads of it and the world as it runs ([`Surroundings`]): the unit,
    /// its master (`master`, its class `class`), its owner `player` (a captured unit's new one,
    /// whose map sees the AI's target), `busy` (an action of its own) and `interruptible` (slot
    /// 0x2d0), through `view` (the owner's side).
    ///
    /// # Errors
    ///
    /// A busy unit of a master the slice does not have, and what the target's block and the
    /// named units do not model.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn surroundings(
        &self,
        c: &Step<'_>,
        view: &OwnerView,
        (master, class): (Option<&Master>, Option<i16>),
        (player, busy, interruptible): (usize, bool, bool),
    ) -> Result<Surroundings, GenieError> {
        Ok(Surroundings {
            position: self.position(),
            hp: self.life.hp,
            speed: self.body.mover().map_or(0.0, |m| m.stats.speed),
            map: c.w.spatial.map,
            dt_seconds: seconds(c.dt),
            mode_1: (c.w.mode_680 == 1),
            busy,
            player,
            class: match class {
                Some(class) => class,
                None if busy => {
                    return Err(GenieError::unsupported(
                        "a busy unit of a master the slice does not have",
                    ));
                }
                None => -1,
            },
            combat_level: master.map_or(0, |m| m.combat_level),
            attack_priority: master.map_or(0, |m| m.attack_priority),
            size: master.map_or([0.0; 2], |m| m.radius),
            los: master.map_or(0.0, |m| m.line_of_sight),
            search_radius: master.map_or(0.0, |m| m.search_radius),
            weapon_range: master.map_or(0.0, |m| m.weapon_range),
            interruptible,
            task_0x99: master.is_some_and(|m| m.tasks.iter().any(|t| t.action_type == 0x99)),
            garrisoned: self.life.inside.is_some(),
            state: self.life.state,
            kind: master.map_or(0, |m| m.kind),
            ranged: master.is_some_and(|m| 0 <= m.missile),
            firepower: self.firepower,
            capture: c
                .w
                .spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(self.id))
                .map_or(0, |s| s.capture),
            master: self.life.master_id,
            scenario: scenario_mode(c.w.settings.game_mode),
            task_0x69: master.is_some_and(|m| m.tasks.iter().any(|t| t.action_type == 0x69)),
            // a new owner sees the target with its own map
            target: match c.ai_target.clone()? {
                Some(t) if view.player != c.player => {
                    let [x, y, _] = t.position;
                    let (tx, ty) = (
                        crate::math::conversions::cvttss2si(x),
                        crate::math::conversions::cvttss2si(y),
                    );
                    Some(crate::unit_ai::Seen {
                        visible: c.w.spatial.visible(view.player, x as i32, y as i32),
                        sight: c.w.spatial.tile_sight(view.player, tx, ty).unwrap_or(0),
                        ..t
                    })
                }
                t => t,
            },
            under_attack: self.under_attack,
            id: self.id,
            anim_speed: self.body.anim_speed(),
            action_state: if self.unmodelled_action.is_some() {
                None
            } else {
                Some(self.top_action().map_or(0, |a| a.state))
            },
            attack_listed: self.actions.attack().is_some(),
            units: self.named_units(
                c,
                changed_view(OthersRef::of(&c.w.players, c.player), view.new_owner),
                view,
                master,
            )?,
            group: c
                .w
                .groups
                .get(self.group.id)
                .map(|g| crate::unit_ai::GroupState {
                    commanded: g.current_command != -1,
                    idle: g.is_idle(),
                    patrol: g.patrols(),
                    moving: g.motion.moving(),
                    member_moving: self.body.mover().is_some_and(Mover::is_moving),
                }),
            top_target: self.top_target_seen(c, OthersRef::of(&c.w.players, c.player)),
            min_range: master.map_or(0.0, |m| m.min_range),
            attack_moving: self.body.mover().is_some_and(|m| m.attack_moving),
            moved_at: {
                let [x, y, height] = self
                    .body
                    .mover()
                    .map_or_else(|| self.position(), |m| m.moved_at);
                [x, height, y]
            },
            dodge_chance: c.w.players.get(player).map_or(0, |q| q.dodge_chance),
            keep_distance_chance: c
                .w
                .players
                .get(player)
                .map_or(0, |q| q.keep_distance_chance),
            disable_tower_priority: view.ai_settings.disable_tower_priority,
            guest_computer: self.garrison.iter().any(|&g| {
                c.w.spatial.get(g).is_some_and(|p| {
                    p.player != player && c.w.players.get(p.player).is_some_and(|q| q.computer)
                })
            }),
        })
    }

    /// Runs `f` on the unit's AI with its side ([`UnitView`]: the unit without its AI, on the
    /// update's step, the owner as `view` names it) and the AI stream, with `attacker` for the
    /// attack tests ([`Unit::attacker_of`]), as player `player` (`computer`); the AI is the
    /// unit's again after it and is posted the notifies the side's calls gave it.
    ///
    /// # Panics
    ///
    /// A unit without a unit AI: the callers ask for one.
    ///
    /// # Errors
    ///
    /// `f`'s, a notify the AI's queue does not take, and a draw from the AI stream in a world
    /// routine the side called while the AI held the stream.
    pub(crate) fn with_side<R>(
        &mut self,
        c: &mut Step<'_>,
        view: &OwnerView,
        attacker: crate::unit_ai::Attacker,
        (player, computer): (usize, bool),
        f: impl FnOnce(&mut UnitAi, &mut UnitView<'_, '_>, &mut Well512) -> Result<R, GenieError>,
    ) -> Result<R, GenieError> {
        let mut ai = self.ai.take().expect("a unit with a unit AI");
        // the AI stream is the AI's while it runs: no world routine the side calls draws from it
        let held = c.w.streams.get(Stream::Ai).clone();
        let mut ai_stream = held.clone();
        let outer = c.side_notifies.replace(Vec::new());
        let mut side = UnitView {
            c: &mut *c,
            unit: self,
            attacker,
            player,
            computer,
            construction_rate: view.construction_rate,
            new_owner: view.new_owner,
            civ: view.civ,
            ai_settings: view.ai_settings,
        };
        let result = f(&mut ai, &mut side, &mut ai_stream);
        let own = std::mem::replace(&mut c.side_notifies, outer).unwrap_or_default();
        let posted = own.into_iter().try_for_each(|(n, state)| ai.post(n, state));
        self.ai = Some(ai);
        let stream = c.w.streams.get(Stream::Ai);
        if *stream != held {
            return Err(GenieError::unreachable(
                "a world routine drew from the AI stream while the unit AI held it",
            ));
        }
        *stream = ai_stream;
        let answer = result?;
        posted?;
        Ok(answer)
    }

    /// The unit's AI run outside its update (a command's immediate order): `f` gets the AI, what
    /// it reads of the unit and the world ([`Unit::surroundings`], as the updating player's own
    /// unit), its side ([`Unit::with_side`], whose calls on the unit run at once), the owner's
    /// computer flag and the AI stream.
    ///
    /// # Errors
    ///
    /// A unit without a unit AI, slot 0x2d0 and the surroundings' parts the slice does not model,
    /// and what `f` and its calls do not.
    pub(crate) fn run_ai(
        &mut self,
        c: &mut Step<'_>,
        f: impl FnOnce(
            &mut UnitAi,
            &Surroundings,
            &mut dyn crate::unit_ai::UnitSide,
            bool,
            &mut Well512,
        ) -> Result<(), GenieError>,
    ) -> Result<(), GenieError> {
        let Some(ai_type) = self.ai.as_ref().map(|ai| ai.ai_type) else {
            return Err(GenieError::unreachable(
                "a unit AI's run of a unit without one",
            ));
        };
        let busy = self.top_action_kind().is_some() || self.unmodelled_action.is_some();
        let master = self.master(&c.w.masters, c.player).ok();
        let class = master.map(|ms| ms.class);
        let (player, computer) = (c.player, c.w.players[c.player].computer);
        let view = OwnerView::of(c, None);
        let interruptible = self.interruptible()?;
        let at = self.surroundings(c, &view, (master, class), (player, busy, interruptible))?;
        let attacker = self.attacker_of(
            ai_type,
            master,
            (player, computer),
            scenario_mode(c.w.settings.game_mode),
            at.capture,
        );
        self.with_side(c, &view, attacker, (player, computer), |ai, side, rng| {
            f(ai, &at, side, computer, rng)
        })
    }

    /// One pass of the unit AI ([`Unit::ai_part`]) as the player `captured` names (its new
    /// owner's for a main capture, else the owner's): on the owner's turn the full update, or with
    /// `after` its rest after an order on a target ([`crate::unit_ai::UnitAi::after_order`]),
    /// else its check between turns, its calls on the unit at once. Returns the rest an order on
    /// a target the full update took leaves ([`AfterOrder`]).
    ///
    /// # Errors
    ///
    /// The unit AI's, its surroundings' and its calls'.
    fn ai_run(
        &mut self,
        c: &mut Step<'_>,
        captured: Option<&Captured>,
        after: Option<AfterOrder>,
    ) -> Result<Option<AfterOrder>, GenieError> {
        let busy = self.top_action_kind().is_some() || self.unmodelled_action.is_some();
        // the unit's master: its own (a captured unit's copy), else its owner's
        let master = object_master(
            self.own_master.as_deref(),
            (self.id, self.life.master_id),
            c.w.masters.get(c.player),
        )
        .ok();
        let class = master.map(|ms| ms.class);
        // a captured unit's unit AI runs as its new owner's (RGE_Combat_Object::update reads the
        // owner then): on that player's turn, with its time as it stands
        let (turn, player, computer) = match captured {
            Some(k) if k.main => (
                (k.new == c.w.player_turn)
                    .then(|| c.w.player_time.get(k.new).copied())
                    .flatten(),
                k.new,
                c.w.players.get(k.new).is_some_and(|q| q.computer),
            ),
            _ => (c.turn, c.player, c.w.players[c.player].computer),
        };
        let view = OwnerView::of(c, captured.filter(|k| k.main).map(|k| k.new));
        // RGE_Combat_Object::update: on the owner's turn the full update (slot 4), else
        // FUN_140c68980 for a unit not inside another; a dead unit's full update only adds the
        // world's dt to its time since the last enemy sighting (`+0x1b4`) before it returns
        if turn.is_some() && self.ai.is_some() && after.is_none() {
            // the full update's start lets the unit AIs' search cache go
            c.w.spatial.forget_search();
        }
        if turn.is_some()
            && after.is_none()
            && !self.life.is_alive()
            && let Some(ai) = self.ai.as_mut()
        {
            ai.time_since_enemy += seconds(c.dt);
        }
        let runs = turn.is_some() || self.life.inside.is_none();
        let Some(orders) = self
            .ai
            .as_ref()
            .filter(|_| self.life.is_alive() && runs)
            .map(|ai| !ai.orders.is_empty())
        else {
            return Ok(None);
        };
        // slot 0x2d0 matters only to a queue with orders
        let interruptible = match self.interruptible() {
            Ok(i) => i,
            Err(e) if orders => return Err(e),
            Err(_) => false,
        };
        let at = self.surroundings(c, &view, (master, class), (player, busy, interruptible))?;
        // the unit AI runs on its own; what it asks of its unit comes back through `UnitView`
        let world_time = c.w.world_time;
        let ai_type = self.ai.as_ref().map_or(0, |ai| ai.ai_type);
        let attacker = self.attacker_of(
            ai_type,
            master,
            (player, computer),
            scenario_mode(c.w.settings.game_mode),
            at.capture,
        );
        self.with_side(
            c,
            &view,
            attacker,
            (player, computer),
            |ai, side, ai_stream| {
                // an order on a target comes only from a full update, on the owner's turn
                match (turn, after) {
                    (Some(_), Some(after)) => ai
                        .after_order(after, world_time, computer, &at, ai_stream, side)
                        .map(|()| None),
                    (Some(dt), None) => ai.update(dt, world_time, computer, &at, ai_stream, side),
                    (None, _) => ai.off_turn(&at, side, computer, ai_stream).map(|()| None),
                }
            },
        )
    }

    /// The objects its unit AI names besides its target, as the world's table holds them
    /// ([`crate::unit_ai::Surroundings::units`]): its attackers and the callers of its damage
    /// notifies, from its own list or another player's, each with its master's class, state, its
    /// unit AI's target and its footprint's squared gap to the unit's (`FUN_14118f8c0`).
    pub(crate) fn named_units(
        &self,
        c: &Step<'_>,
        others: OthersRef<'_>,
        view: &OwnerView,
        master: Option<&Master>,
    ) -> Result<Vec<crate::unit_ai::UnitSeen>, GenieError> {
        let Some(ai) = self.ai.as_ref() else {
            return Ok(Vec::new());
        };
        let damaged = ai
            .notifies
            .iter()
            .filter(|n| n.kind == crate::unit_ai::DAMAGED)
            .map(|n| n.caller);
        let in_the_way = ai
            .notifies
            .iter()
            .filter(|n| n.kind == crate::unit_ai::IN_THE_WAY)
            .map(|n| n.p[0]);
        let mut ids: Vec<i32> = ai
            .attackers
            .iter()
            .copied()
            .chain(damaged)
            .chain(in_the_way)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let mut seen = Vec::new();
        for id in ids {
            let found = others.find(id);
            let (Some((owner, u)), Some(p)) = (found, c.w.spatial.get(id)) else {
                continue;
            };
            let m = u.master(&c.w.masters, owner)?;
            let me = master.ok_or(crate::world::UNIT_AI_MASTER)?;
            seen.push(crate::unit_ai::UnitSeen {
                id,
                class: m.class,
                owner,
                combat_level: m.combat_level,
                state: u.life.state,
                target: u.ai.as_ref().map(|a| a.current_target),
                gap_squared: footprint_gap_squared(
                    self.position(),
                    me.radius,
                    p.position,
                    p.radius,
                ),
                position: p.position,
                hp: u.life.hp,
                master: u.life.master_id,
                kind: m.kind,
                main: u.annex_of,
                allied: crate::unit_ai::task_finder::allied(
                    view.relations(c.w),
                    view.player,
                    owner,
                    true,
                ),
            });
        }
        Ok(seen)
    }

    /// A notify the unit posts to its own unit AI (slot 0x328).
    pub(crate) fn post_own(&mut self, kind: i32, p: [i32; 3]) -> Result<(), GenieError> {
        let (id, state) = (self.id, self.life.state);
        self.ai
            .as_mut()
            .map_or(Ok(()), |ai| ai.post(Notify::own(id, kind, p), state))
    }

    /// The units in the way the motion step told the unit AI of ([`crate::unit_ai::IN_THE_WAY`],
    /// [`crate::movement::Mover::in_the_way`]), posted at once in their order (slot 0x328: the
    /// walker is in state 2).
    ///
    /// # Errors
    ///
    /// The notifies'.
    pub(crate) fn post_in_the_way(&mut self) -> Result<(), GenieError> {
        let in_the_way = self
            .body
            .mover_mut()
            .map(|m| std::mem::take(&mut m.in_the_way))
            .unwrap_or_default();
        for id in in_the_way {
            self.post_own(crate::unit_ai::IN_THE_WAY, [id, -1, -1])?;
        }
        Ok(())
    }
}
