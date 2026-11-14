//! An attack's start, a hunt's and an action's retarget.

use crate::combat::attack::BeginLayer;
use crate::combat::attack::{Attack, AttackReads, Fight};
use crate::construction::MainBuildingId;
use crate::gathering::hunt::{HuntAction, HuntRetarget, NewTarget, Retargeted, stand_sprite};
use crate::map::Distance;
use crate::map::Position;
use crate::math::rng::Stream;
use crate::movement::MoveStart;
use crate::objects::ListIndex;
use crate::objects::ObjectId;
use crate::objects::Voice;
use crate::objects::actions::LAST_TAKEN;
use crate::objects::masters::ClassId;
use crate::objects::masters::FogVisibility;
use crate::objects::masters::MasterKind;
use crate::objects::teardown::HeadStart;
use crate::players::PlayerId;
use crate::unit_ai::Notifies;
use crate::world::WorldTime;
use crate::world::slots::PlayerSlotsExt;
use crate::{GenieError, GenieResult};

use crate::combat::AttackTargetId;
use crate::combat::attack::AttackSource;
use crate::combat::attack::{
    OwnStart, ReadyReads, arms, engagement, show_arrived, target_in_sight,
};
use crate::combat::victims::victim_of;
use crate::combat::victims::{ChargeOf, VictimLayers, VictimView};
use crate::gathering::gather::unit::WalkSprites;
use crate::objects::masters::player_masters;
use crate::objects::masters::{Master, MasterTables, Task};
use crate::objects::sprite::new_sprite;
use crate::unit_ai::AiAction;
use crate::unit_ai::Notify;
use crate::unit_ai::OrderKind;
use crate::world::World;
use crate::world::{ATTACKER_UNPLACED, HUNTER_UNMOVING, NO_STANDING_SPRITE, TARGET_UNPLACED};

/// Where an attack task's target aims ([`World::attack_aim`]).
pub(super) struct Aim {
    /// The target's owner.
    pub(super) owner: PlayerId,
    /// The target's master's class and fog visibility.
    pub(super) class: ClassId,
    pub(super) fog_visibility: FogVisibility,
    /// The object aimed at (the target, or a building annex's main building), its place and its
    /// master's class.
    pub(super) aimed: ObjectId,
    pub(super) at: Position,
    pub(super) aimed_class: ClassId,
}

impl World {
    /// What the attack's first update reads of the attacker at `idx` of player `player` and of
    /// its `target` as combat objects; none when the world cannot read the target.
    fn attack_ready(
        &self,
        player: PlayerId,
        idx: ListIndex,
        target: ObjectId,
        (master, world_time): (&Master, WorldTime),
    ) -> GenieResult<Option<ReadyReads>> {
        let unit = self.players.live(player, idx)?;
        let held = self.players.player(player)?;
        let reads = AttackReads::of(
            AttackSource {
                combat: self.parts.combat_of(unit.id),
                top_speed: self.top_speed_terms(player, unit)?,
                master,
                graphics: &self.model.graphics,
                ai: self.parts.unit_ai.get(unit.id),
                player_kind: held.kind,
                now: world_time,
            },
            &self.model.roles,
            &self.model.tuning.turn,
        );
        let own = victim_of(
            VictimView {
                unit,
                layers: VictimLayers::of(&self.parts, unit.id, self.parts.actions.of(unit.id)),
                master,
                charge_of: ChargeOf(&|with| self.special_charge(unit, with)),
                player,
                masters: player_masters(&self.model.masters, player)?,
                attributes: &held.attributes,
                terrain: self.spatial.terrain.as_ref(),
                mode_680: self.mode_680,
            },
            &self.model.roles,
        )?;
        Ok(self.victim(target, player)?.map(|victim| ReadyReads {
            reads,
            fight: Fight { victim, own },
        }))
    }

    /// The work command's Attack (its task of action type 7, the factory `FUN_140a97e00` case 7):
    /// the unit AI takes action 600 (`FUN_140c64b50`) and the target (an annex its main building,
    /// `FUN_141189b20`) at its place (`FUN_140c64c10`); the Attack (`FUN_140a7c210`, sub-action 0:
    /// the task's sprites, the task's range or slot 0x238's for 0) takes its target (slot 0x178);
    /// the notify (`FUN_14106e4e0` kind 9: the last retarget time `+0xbc` takes the world's time,
    /// then order 700, action 600, the target with its class and the work's point); the install
    /// clears the action list and starts the Attack (slot 5, `0x140a7f8c0` with 1: `+0x900`
    /// cleared, the standing sprite, the range test of [`Attack::start_own`]; a unit that cannot move in reach
    /// goes ready, `set_state(6)`, [`Attack::enter_ready`]; one that may move ends in
    /// `set_state(4)`, whose `MoveTo` starting arrived shows the standing sprite at its speed and
    /// clears `+0x7b8`, `0x140aa29e0`); then the Attack takes the target as
    /// its second target too (`FUN_140a7a190`, slot 0x178). The unit AI's `+0x1d5`, cleared, is
    /// not held; a task range other than 0 is not modelled.
    pub(super) fn start_attack(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        target: ObjectId,
        task: &Task,
        point: Position,
        notifies: Notifies,
    ) -> GenieResult<()> {
        if task.work_range != Distance::ZERO {
            return Err(GenieError::unsupported(
                "an attack task with a range of its own",
            ));
        }
        let aim = self.attack_aim(target)?;
        let world_time = self.world_time;
        self.attack_notifies(player, idx, (target, &aim), point, notifies)?;
        self.clear_actions(player, idx)?;
        let master = self
            .players
            .live(player, idx)?
            .master(self.master_tables(), player)?;
        // the task factory (`FUN_140a97e00` case 7) gives the Attack the task's moving sprite
        let arms = arms(
            master,
            &self.model.graphics,
            task.sprites,
            task.sprites.moving,
            &self.model.roles,
        );
        let standing = arms.idle_sprite.ok_or(NO_STANDING_SPRITE)?;
        let me_at = *self
            .spatial
            .get(self.players.live(player, idx)?.id)
            .ok_or(ATTACKER_UNPLACED)?;
        let target_at = *self.spatial.get_known(target)?.ok_or(TARGET_UNPLACED)?;
        let visible = target_in_sight(&self.spatial, player, &target_at, aim.fog_visibility)?;
        let ready = self.attack_ready(player, idx, target, (master, world_time))?;
        let attacker = self.players.live(player, idx)?;
        let speed = self.top_speed(player, attacker)?;
        let engagement = engagement(
            self.parts.unit_ai.get(attacker.id),
            speed,
            master,
            &me_at,
            &target_at,
            world_time,
            self.parts.combat_of(attacker.id).next_volley,
        )?;
        let unit = self.players.live_mut(player, idx)?;
        if unit.sprite.is_none() {
            return Err(GenieError::unsupported(
                "an attack task of a unit whose sprite the slice does not hold",
            )
            .with("object", unit.id)
            .with("master", unit.life.master_id));
        }
        new_sprite(
            &mut unit.sprite,
            self.parts.move_stats_mut(unit.id),
            standing,
            self.streams.get_mut(Stream::Sprite),
            self.model.tuning.sprite,
        );
        let start = OwnStart {
            target,
            engagement: &engagement,
            visible,
            target_owner: aim.owner,
            ready,
        };
        self.install_attack(player, idx, Attack::own(target, &engagement, arms), &start)
    }

    /// The new `attack`'s start on object `idx` of `player` ([`Attack::start_own`] with 1 and
    /// `start`): the start's notifies go to the unit AI, the Attack takes its target as its second
    /// target too (`FUN_140a7a190`) and goes on the action list, and the unit takes the target
    /// (slot 0x178).
    fn install_attack(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        mut attack: Attack,
        start: &OwnStart<'_>,
    ) -> GenieResult<()> {
        let roles = self.model.roles.clone();
        let target = start.target;
        let posted = self.run_on_step(player, idx, |unit, _, step| {
            attack.start_own(step, unit, HeadStart::Added, start, &roles)
        })?;
        // the start's notifies go to the unit AI (the unit's slot 0x328 -> the AI's slot 0x38)
        let unit = self.players.live_mut(player, idx)?;
        let (id, state) = (unit.id, unit.life.state);
        for (kind, params) in posted {
            if let Some(ai) = self.parts.unit_ai.get_mut(unit.id) {
                ai.post(Notify::own(id, kind, params), state)?;
            }
        }
        // the attack command gives the Attack its second target
        attack.target2 = Some(AttackTargetId::new(target));
        self.parts
            .actions
            .of_mut(unit.id)?
            .list
            .set_attack(Some(attack));
        self.take(target, self.releaser(player, idx)?)?;
        Ok(())
    }

    /// Where an attack task on `target` aims (`FUN_141189b20`): a building annex at its main
    /// building, anything else at itself; with the target's owner, its master's class and fog
    /// visibility, and the aimed object's place and class.
    pub(super) fn attack_aim(&self, target: ObjectId) -> GenieResult<Aim> {
        let (owner, target_unit) = self.object(target).ok_or(GenieError::unsupported(
            "an attack task's target the slice does not hold",
        ))?;
        let tm = target_unit.master(self.master_tables(), owner)?;
        // A building's master type (`'P'`); types below 30 do not move.
        let aimed = if tm.kind == MasterKind::BUILDING {
            self.parts
                .building_of(target_unit.id)
                .annex_of
                .map_or(target, MainBuildingId::id)
        } else {
            target
        };
        let (ap, unit) = self.object(aimed).ok_or(GenieError::unsupported(
            "an attack task's main building is gone",
        ))?;
        Ok(Aim {
            owner,
            class: tm.class,
            fog_visibility: tm.fog_visibility,
            aimed,
            at: unit.position(),
            aimed_class: unit.master(self.master_tables(), ap)?.class,
        })
    }

    /// The attack task's word to the unit AI of object `idx` of `player`: action 600 and the
    /// aimed object at its place (`FUN_140c64b50`, `FUN_140c64c10`); the target taken (slot
    /// 0x178); the last retarget time the world's; the work notify (`FUN_14106e4e0` kind 9:
    /// order 700, action 600, the target with its class and the work's `point`, its notifies
    /// cleared with `notifies`) and the order history.
    pub(super) fn attack_notifies(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        (target, aim): (ObjectId, &Aim),
        point: Position,
        notifies: Notifies,
    ) -> GenieResult<()> {
        let world_time = self.world_time;
        let unit = self.players.live_mut(player, idx)?;
        if let Some(ai) = self.parts.unit_ai.get_mut(unit.id) {
            ai.set_action(Some(AiAction::Attack));
            ai.set_target(Some(aim.aimed), aim.aimed_class.or_none(), aim.at);
        }
        self.take(target, self.releaser(player, idx)?)?;
        let unit = self.players.live_mut(player, idx)?;
        if let Some(ai) = self.parts.unit_ai.get_mut(unit.id) {
            ai.last_retarget_time = world_time;
            ai.notify_work(
                Some(OrderKind::Attack),
                Some(AiAction::Attack),
                Some((target, aim.class)),
                point,
                notifies,
                self.streams.get_mut(Stream::Ai),
            )?;
            self.remember_order(
                player,
                idx,
                (Some(OrderKind::Attack), Some(AiAction::Attack)),
            )?;
        }
        Ok(())
    }

    /// The action list's top action's slot 0x48 (`FUN_140a977a0`) on the Hunt `hunt` that the
    /// caller found at the head of the hunter at `idx`: its slot 9 ([`HuntAction::retarget`],
    /// `0x140ac97c0`) on the new `target` of the work command or the hunt command (none for
    /// none) with `point`. The Hunt goes back to the head. A Hunt whose Attack's `MoveTo`
    /// starts arrived shows its idle sprite; one that failed at once the sprite it stands with
    /// (`set_state(13)`'s slot 0x68 with `FUN_140ac5210`'s, [`stand_sprite`]). Then the targets
    /// that changed are let go and taken ([`World::exchange_targets`]).
    ///
    /// # Errors
    ///
    /// What [`World::hunt_new_target`] cannot read; a hunter without its mover; what the Hunt's
    /// walks and Attack do not model.
    ///
    /// [`HuntAction::retarget`]: crate::gathering::hunt::HuntAction::retarget
    pub(super) fn retarget_hunt(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        mut hunt: HuntAction,
        target: Option<ObjectId>,
        point: Position,
    ) -> GenieResult<()> {
        let new = match target {
            Some(target) => Some(self.hunt_new_target(player, idx, target)?),
            None => None,
        };
        let arms = self.arms(player, idx, &hunt.task)?;
        let by = self.releaser(player, idx)?;
        let unit = self.players.live(player, idx)?;
        let top_speed = self.top_speed_terms(player, unit)?;
        // FUN_140ac5210 reads the unit's load, which the retarget leaves as it is
        let stand = stand_sprite(
            &hunt.task,
            unit.master(self.master_tables(), player)?.standing_sprite,
            unit.life.held(),
            &self.model.graphics,
        );
        let unit = self.players.live_mut(player, idx)?;
        let walks = unit.move_walks(
            self.parts.mover_of(unit.id),
            top_speed,
            WalkSprites {
                every: MasterTables::new(&self.model.masters, &self.parts.own_master),
                player,
                graphics: &self.model.graphics,
                look: self.parts.combat_of(unit.id).abilities.look.as_ref(),
            },
            &self.model.roles,
        )?;
        let mover = self
            .parts
            .motion
            .mover_mut(unit.id)
            .ok_or(HUNTER_UNMOVING)?;
        let old = hunt.targets();
        let retargeted = hunt.retarget(
            HuntRetarget {
                target: new.as_ref(),
                point,
                player,
                walks,
            },
            arms,
            mover,
            BeginLayer {
                roles: &self.model.roles,
                attack_tuning: &self.model.tuning.attack,
                move_to_tuning: &self.model.tuning.move_to,
            },
            &self.model.tuning.hunt,
        );
        self.parts
            .actions
            .of_mut(unit.id)?
            .list
            .set_hunt(Some(hunt));
        match retargeted? {
            Retargeted::Started(MoveStart::Arrived) => show_arrived(
                &arms,
                &mut unit.sprite,
                self.parts.motion.layer_mut(unit.id),
                self.streams.get_mut(Stream::Sprite),
                self.model.tuning.sprite,
            )?,
            Retargeted::Failed => {
                if let Some(graphic) = stand {
                    new_sprite(
                        &mut unit.sprite,
                        self.parts.move_stats_mut(unit.id),
                        graphic,
                        self.streams.get_mut(Stream::Sprite),
                        self.model.tuning.sprite,
                    );
                }
            }
            Retargeted::Kept | Retargeted::Started(MoveStart::UnderWay) => {}
        }
        self.exchange_targets(old, hunt.targets(), by)
    }

    /// What the Hunt's slot 9 reads of the new `target` of the hunter at `idx` ([`NewTarget`]):
    /// the target ([`World::quarry`]), the hunter's engagement with it, and whether the target's
    /// own master (`+0x58`, not an annex's main building's) is one of the hunter's master's drop
    /// sites (`FUN_14109d4f0`).
    ///
    /// # Errors
    ///
    /// A target the world does not hold or the hunter cannot reach; a building that the load
    /// could not read whole (its [`Unread::error`]): slot 9 reads its action list's top action.
    ///
    /// [`NewTarget`]: crate::gathering::hunt::NewTarget
    /// [`Unread::error`]: crate::world::Unread::error
    fn hunt_new_target(
        &self,
        player: PlayerId,
        idx: ListIndex,
        target: ObjectId,
    ) -> GenieResult<NewTarget> {
        let quarry = self.quarry(player, target)?;
        if quarry.kind == MasterKind::BUILDING
            && let Some(unread) = self.parts.unread.get(target)
        {
            return Err(unread.error());
        }
        let target_master = self
            .object(target)
            .map(|(_, target_unit)| target_unit.life.master_id);
        let drops_here = self
            .players
            .live(player, idx)?
            .master(self.master_tables(), player)?
            .action_part()
            .drop_sites
            .iter()
            .any(|&master_id| Some(master_id) == target_master);
        Ok(NewTarget {
            quarry,
            engagement: self.engagement(player, idx, target)?,
            drops_here,
        })
    }

    /// An action's slot 0x68 base (`FUN_140a7a230`) for the unit at `idx`: its target `old`
    /// becomes `new`. The same target changes nothing. A different old target the
    /// world still holds is let go (slot 0x180, [`World::release`]) unless the action is a
    /// quiet sub-action (`voice`, `+0x60`); a new target in state 7 or more is not taken, and
    /// the action is left without one; else it is taken (slot 0x178 with the action's type,
    /// [`World::take`]) unless quiet. Returns the action's target (`+0x30`) after it.
    pub(crate) fn retarget_action(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        old: Option<ObjectId>,
        new: ObjectId,
        voice: Voice,
    ) -> GenieResult<Option<ObjectId>> {
        if old == Some(new) {
            return Ok(old);
        }
        if let Some(old) = old
            && voice.is_loud()
            && self.object(old).is_some()
        {
            let by = self.releaser(player, idx)?;
            self.release(old, by)?;
        }
        let Some((_, unit)) = self.object(new) else {
            return Ok(None);
        };
        if LAST_TAKEN < unit.life.state {
            return Ok(None);
        }
        if voice.is_loud() {
            self.take(new, self.releaser(player, idx)?)?;
        }
        Ok(Some(new))
    }
}
