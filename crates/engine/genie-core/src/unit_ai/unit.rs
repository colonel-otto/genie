//! The unit AI's part of the update: its surroundings, its side and its run.

use crate::math::rng::{Stream, Well512};
use crate::movement::Mover;
use crate::objects::masters::Master;
use crate::objects::unit::{Captured, Step, Unit};
use crate::unit_ai::world_view::{Context, OwnerView, UnitView};
use crate::unit_ai::{Effect, GenieError, Surroundings, UnitAi};

impl Unit {
    /// The end of [`Unit::update`] after the actions: `RGE_Action_Object::update`'s end
    /// (`FUN_141163080`: the action object's shape), `RGE_Combat_Object::update`'s sight mask,
    /// then the unit AI's full update on its owner's turn, else its check between turns, and the
    /// effects it has on the unit; then the step the update ends with.
    pub(crate) fn ai_part(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
        resume: AiResume,
    ) -> Result<Step, GenieError> {
        let AiResume {
            captured,
            leaves,
            after,
        } = resume;
        // the rest of a full update after its order on a target ran its part before that already
        if after.is_none() {
            self.refresh_pathing(c);
        }
        // RGE_Combat_Object::update after the actions: the object's sight mask; a changed mask of
        // an object others take over by seeing it (`+0x924` 1) gives it to the first player that
        // sees it ([`crate::vision::visibility::sight_taker`]) when that one has its master
        let mut captured = captured;
        if after.is_none()
            && self.combat
            && let Some(mask) = c.spatial.combat_sight(self.id)?
            && let Some(q) =
                crate::vision::visibility::sight_taker(mask, c.player, c.standard, c.sight_takers)
            && c.every_masters
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
        let busy = self.top_action_kind().is_some() || self.unmodelled_action.is_some();
        // the unit's master: its own (a captured unit's copy), else its owner's
        let master = self
            .own_master
            .as_deref()
            .or_else(|| c.masters.and_then(|ms| ms.get(self.life.master_id)));
        let class = master.map(|ms| ms.class);
        // a captured unit's unit AI runs as its new owner's (RGE_Combat_Object::update reads the
        // owner then): on that player's turn, with its time as it stands
        let (turn, player, computer) = match &captured {
            Some(k) if k.main => (
                (k.new == c.player_turn)
                    .then(|| c.player_time.get(k.new).copied())
                    .flatten(),
                k.new,
                c.computers.get(k.new).copied().unwrap_or(false),
            ),
            _ => (c.turn, c.player, c.computer),
        };
        let view = OwnerView::of(
            c,
            others,
            captured.as_ref().filter(|k| k.main).map(|k| k.new),
        );
        // RGE_Combat_Object::update: on the owner's turn the full update (slot 4), else
        // FUN_140c68980 for a unit not inside another; a dead unit's full update only adds the
        // world's dt to its time since the last enemy sighting (`+0x1b4`) before it returns
        if turn.is_some() && self.ai.is_some() && after.is_none() {
            // the full update's start lets the unit AIs' search cache go
            c.spatial.forget_search();
        }
        if turn.is_some()
            && !self.life.is_alive()
            && let Some(ai) = self.ai.as_mut()
        {
            ai.time_since_enemy += c.dt_seconds;
        }
        let runs = turn.is_some() || self.life.inside.is_none();
        if let Some(orders) = self
            .ai
            .as_ref()
            .filter(|_| self.life.is_alive() && runs)
            .map(|ai| !ai.orders.is_empty())
        {
            // slot 0x2d0 matters only to a queue with orders
            let interruptible = match self.interruptible() {
                Ok(i) => i,
                Err(e) if orders => return Err(e),
                Err(_) => false,
            };
            let at = self.surroundings(
                c,
                others,
                &view,
                (master, class),
                (player, busy, interruptible),
            )?;
            // the unit AI runs on its own; what it asks of its unit comes back through `UnitView`
            let world_time = c.world_time;
            let ai_type = self.ai.as_ref().map_or(0, |ai| ai.ai_type);
            let attacker =
                self.attacker_of(ai_type, master, (player, computer), c.scenario, at.capture);
            let effects = self.with_side(
                c,
                &view,
                attacker,
                (player, computer),
                |ai, side, ai_stream| {
                    // an order on a target comes only from a full update, on the owner's turn
                    match (turn, after) {
                        (Some(_), Some(after)) => {
                            ai.after_order(after, world_time, computer, &at, ai_stream, side)
                        }
                        (Some(dt), None) => {
                            ai.update(dt, world_time, computer, &at, ai_stream, side)
                        }
                        (None, _) => ai.off_turn(&at, side, computer, ai_stream),
                    }
                },
            )?;
            self.apply(effects, class, c)?;
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
            return Ok(Step::Captured(k));
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
        c: &Context<'_>,
        others: &[Self],
        view: &OwnerView<'_>,
        (master, class): (Option<&Master>, Option<i16>),
        (player, busy, interruptible): (usize, bool, bool),
    ) -> Result<Surroundings, GenieError> {
        Ok(Surroundings {
            position: self.position,
            hp: self.life.hp,
            speed: self.mover.as_ref().map_or(0.0, |m| m.stats.speed),
            map: c.spatial.map,
            dt_seconds: c.dt_seconds,
            mode_1: c.mode_1,
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
                .spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(self.id))
                .map_or(0, |s| s.capture),
            master: self.life.master_id,
            scenario: c.scenario,
            task_0x69: master.is_some_and(|m| m.tasks.iter().any(|t| t.action_type == 0x69)),
            // a new owner sees the target with its own map
            target: match c.ai_target.clone()? {
                Some(t) if view.player != c.player => {
                    let [x, y, _] = t.position;
                    let (tx, ty) = (
                        crate::world::checkpoint::cvttss2si(x),
                        crate::world::checkpoint::cvttss2si(y),
                    );
                    Some(crate::unit_ai::Seen {
                        visible: c.spatial.visible(view.player, x as i32, y as i32),
                        sight: c.spatial.tile_sight(view.player, tx, ty).unwrap_or(0),
                        ..t
                    })
                }
                t => t,
            },
            under_attack: self.under_attack,
            id: self.id,
            anim_speed: self.anim_speed,
            action_state: if self.unmodelled_action.is_some() {
                None
            } else {
                Some(self.top_action().map_or(0, |a| a.state))
            },
            attack_listed: self.attack.is_some(),
            units: self.named_units(c, view, master)?,
            group: c
                .groups
                .get(self.group.id)
                .map(|g| crate::unit_ai::GroupState {
                    commanded: g.current_command != -1,
                    idle: g.is_idle(),
                    patrol: g.patrols(),
                    moving: g.motion.moving(),
                    member_moving: self.mover.as_ref().is_some_and(Mover::is_moving),
                }),
            top_target: self.top_target_seen(c, others),
            min_range: master.map_or(0.0, |m| m.min_range),
            attack_moving: self.mover.as_ref().is_some_and(|m| m.attack_moving),
            moved_at: {
                let [x, y, height] = self.mover.as_ref().map_or(self.position, |m| m.moved_at);
                [x, height, y]
            },
            dodge_chance: c.dodge_chances.get(player).copied().unwrap_or(0),
            keep_distance_chance: c.keep_distance_chances.get(player).copied().unwrap_or(0),
            disable_tower_priority: view.ai_settings.disable_tower_priority,
            guest_computer: self.garrison.iter().any(|&g| {
                c.spatial.get(g).is_some_and(|p| {
                    p.player != player && c.computers.get(p.player).copied().unwrap_or(false)
                })
            }),
        })
    }

    /// Runs `f` on the unit's AI with its side ([`UnitView`]: the unit without its AI, what it
    /// reads of the world through `view`, the Object and Sprite streams) and the AI stream, with
    /// `attacker` for the attack tests ([`Unit::attacker_of`]), as player `player`
    /// (`computer`); the AI is the unit's again after it.
    ///
    /// # Panics
    ///
    /// A unit without a unit AI: the callers ask for one.
    pub(crate) fn with_side<R>(
        &mut self,
        c: &mut Context<'_>,
        view: &OwnerView<'_>,
        attacker: crate::unit_ai::Attacker,
        (player, computer): (usize, bool),
        f: impl FnOnce(&mut UnitAi, &mut UnitView<'_>, &mut Well512) -> Result<R, GenieError>,
    ) -> Result<R, GenieError> {
        let mut ai = self.ai.take().expect("a unit with a unit AI");
        let [ai_stream, object, sprite] =
            c.streams.three(Stream::Ai, Stream::Object, Stream::Sprite);
        let result = {
            let mut side = UnitView {
                unit: self,
                attacker,
                player,
                computer,
                spatial: c.spatial,
                pathing: c.pathing,
                groups: c.groups,
                object,
                sprite,
                world_time: c.world_time,
                construction_rate: view.construction_rate,
                others: view.others,
                zones: c.zones,
                foreign: view.foreign,
                every: c.every_masters,
                masters: view.masters,
                graphics: c.graphics,
                takes: &mut c.takes,
                releases: &mut c.releases,
                builder_leaves: &mut c.builder_leaves,
                starts: &mut c.starts,
                attribute_adds: &mut c.attribute_adds,
                promotions: &mut c.promotions,
                relations: view.relations,
                attributes: view.attributes,
                civ: view.civ,
                ai_settings: view.ai_settings,
                drop_sites: c.drop_sites,
                tracker: &*c.tracker,
                mode_680: c.mode_680,
                carry_rule: c.carry_rule,
            };
            f(&mut ai, &mut side, ai_stream)
        };
        self.ai = Some(ai);
        result
    }

    /// The unit's AI run outside its update (a command's immediate order): `f` gets the AI, what
    /// it reads of the unit and the world ([`Unit::surroundings`], as the updating player's own
    /// unit), its side ([`Unit::with_side`]), the owner's computer flag and the AI stream; its
    /// effects then apply as the AI part's do ([`Unit::apply`]).
    ///
    /// # Errors
    ///
    /// A unit without a unit AI, slot 0x2d0 and the surroundings' parts the slice does not model,
    /// and what `f` and the effects do not.
    pub(crate) fn run_ai(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
        f: impl FnOnce(
            &mut UnitAi,
            &Surroundings,
            &mut dyn crate::unit_ai::UnitSide,
            bool,
            &mut Well512,
        ) -> Result<Vec<Effect>, GenieError>,
    ) -> Result<(), GenieError> {
        let Some(ai_type) = self.ai.as_ref().map(|ai| ai.ai_type) else {
            return Err(GenieError::unreachable(
                "a unit AI's run of a unit without one",
            ));
        };
        let busy = self.top_action_kind().is_some() || self.unmodelled_action.is_some();
        let master = self
            .own_master
            .as_deref()
            .or_else(|| c.masters.and_then(|ms| ms.get(self.life.master_id)));
        let class = master.map(|ms| ms.class);
        let (player, computer) = (c.player, c.computer);
        let view = OwnerView::of(c, others, None);
        let interruptible = self.interruptible()?;
        let at = self.surroundings(
            c,
            others,
            &view,
            (master, class),
            (player, busy, interruptible),
        )?;
        let attacker =
            self.attacker_of(ai_type, master, (player, computer), c.scenario, at.capture);
        let effects = self.with_side(c, &view, attacker, (player, computer), |ai, side, rng| {
            f(ai, &at, side, computer, rng)
        })?;
        self.apply(effects, class, c)
    }
}

/// What a unit's update hands its unit AI part when that runs after the caller gave the work its
/// builder found ([`Unit::ai_part`]): a takeover and the step the update ends with; or, with
/// `after`, the rest of the unit AI's full update once the caller gave the order on a target
/// the update took ([`crate::unit_ai::UnitAi::after_order`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AiResume {
    pub(crate) captured: Option<Captured>,
    pub(crate) leaves: Step,
    pub(crate) after: Option<crate::unit_ai::AfterOrder>,
}
