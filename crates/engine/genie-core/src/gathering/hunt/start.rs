//! The Hunt's start, its chase and retarget, a move's target and the walk to a point (states
//! 11 and 15).

use crate::combat::attack::BeginLayer;
use crate::combat::attack::{Arms, Attack, Engagement};
use crate::gathering::DropSiteId;
use crate::gathering::PastureId;
use crate::gathering::gather::Walks;
use crate::gathering::hunt::HuntState;
use crate::gathering::hunt::{HuntAction, HuntTask, Lead, Quarry};
use crate::gathering::{HuntTargetId, load_for_task};
use crate::map::Distance;
use crate::map::Position;
use crate::movement::MoveStart;
use crate::movement::move_to::GiveUp;
use crate::movement::move_to::MoveToAction;
use crate::movement::move_to::Patience;
use crate::movement::move_to::SubCode;
use crate::movement::move_to::WalkTo;
use crate::movement::{Me, Mover};
use crate::objects::ObjectId;
use crate::objects::ObjectState;
use crate::objects::actions::LAST_TAKEN;
use crate::objects::lifecycle::Held;
use crate::objects::unit::Unit;
use crate::players::Amount;
use crate::players::PlayerId;
use crate::players::PlayerKind;
use crate::unit_ai::Answer;
use crate::unit_ai::UnitStance;
use crate::world::Seconds;
use crate::world::layer::ClassRoles;
use crate::world::layer::generated::HuntTuning;
use crate::world::layer::generated::MoveToTuning;
use crate::world::settings::Settings;
use crate::world::step::Step;
use crate::{GenieError, GenieResult};

/// Who walks to a hunt's point ([`HuntAction::update_point_walk`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointWalker {
    /// The owner's type (`player+0x10`; [`PlayerKind::is_computer`] is `FUN_141659cf0`).
    pub(crate) player_kind: PlayerKind,
    /// The unit is a villager (class 4): its `MoveTo` waits once when the motion step gives up.
    pub(crate) villager: bool,
    /// Its unit AI's stance (`+0x1a9`), 0 without one: the wait is a second below 2, else two.
    pub(crate) stance: UnitStance,
}

/// What a hunt's walk to its point did in an update ([`HuntAction::update_point_walk`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointWalk {
    /// It walks on, or waits to ask again.
    Walking,
    /// State 15's walk ended or failed: the hunt is done ([`HuntAction::set_done`]), its target
    /// and second target to release.
    Done {
        /// The target it let go, if any.
        target: Option<ObjectId>,
        /// The second target it let go, if any.
        second: Option<ObjectId>,
    },
    /// State 11's walk ended or failed: the hunt searches (`set_state(3)`; the caller tells the
    /// unit AI).
    Search,
}

/// What a hunt's start takes besides its attack and its mover ([`HuntAction::start`]'s arguments).
#[derive(Debug)]
pub struct HuntStart<'world> {
    /// The target the Hunt holds.
    pub(crate) target: &'world Quarry,
    /// The owner.
    pub(crate) player: PlayerId,
    /// The owner's type (`player+0x10`; [`PlayerKind::is_computer`] is `FUN_141659cf0`).
    pub(crate) player_kind: PlayerKind,
    /// The game settings, whose test `FUN_141302150` ([`Settings::carry_rule`]) the load reads.
    pub(crate) settings: &'world Settings,
    /// What the hunter carries, its resource kind and amount.
    pub(crate) held: &'world mut Held,
    /// What its `MoveTo`s' constructors read of the hunter; its load is `held`'s.
    pub(crate) walks: Walks<'world>,
}

/// A hunt's new target and what its retarget reads with it ([`HuntRetarget::target`]).
#[derive(Clone, Copy, Debug)]
pub struct NewTarget {
    /// The object (`RDX`).
    pub(crate) quarry: Quarry,
    /// The hunter's engagement with it, which its Attack starts with.
    pub(crate) engagement: Engagement,
    /// Its own master (`+0x58` of its `+0xa0`, not an annex's main building's) is one of the
    /// hunter's master's drop sites (`FUN_14109d4f0`).
    pub(crate) drops_here: bool,
}

/// What a hunt's retarget takes besides its arms and its mover ([`HuntAction::retarget`]'s
/// arguments).
#[derive(Clone, Copy, Debug)]
pub struct HuntRetarget<'world> {
    /// The new target, none for none: the work command's on a terrain's task (`R14` null at
    /// `0x1410707ab`).
    pub(crate) target: Option<&'world NewTarget>,
    /// The slot's point (`XMM2`, `XMM3` and `[RSP+0x28]`): the target's place at the hunt command
    /// and at the hunt's own searches, the work command's point.
    pub(crate) point: Position,
    /// The owner.
    pub(crate) player: PlayerId,
    /// What the hunter carries, and what its `MoveTo`s' constructors read of it.
    pub(crate) walks: Walks<'world>,
}

/// What a hunt's retarget did ([`HuntAction::retarget`]); slot 9 answers 1 for each
/// (`MOV AL, 1` at `0x140ac99ed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Retargeted {
    /// The target it holds already (`+0x20`): nothing changed.
    Kept,
    /// It took the point, the drop site or the target, and walks or attacks: how its walk or its
    /// Attack's `MoveTo` started.
    Started(MoveStart),
    /// `set_state(4)` failed it at once (`set_state(13)`, `LAB_140ac679d`): it holds no target,
    /// or the target is a live allied one. The unit takes the sprite the hunt stands with (case
    /// 0xd's slot 0x68 with `FUN_140ac5210`'s, [`crate::gathering::hunt::stand_sprite`]), the
    /// caller's.
    Failed,
}

/// What a hunt's chase of its target takes besides its attack and its mover
/// ([`HuntAction::chase`]'s arguments).
#[derive(Clone, Copy, Debug)]
pub struct HuntChase<'world> {
    /// The target.
    pub(crate) target: &'world Quarry,
    /// The hunter's owner.
    pub(crate) player: PlayerId,
    /// What its `MoveTo`s' constructors read of the hunter.
    pub(crate) walks: &'world Walks<'world>,
}

/// A move a hunt may take ([`HuntAction::take_move`]'s arguments before the mover).
#[derive(Clone, Copy, Debug)]
pub struct HuntMove<'world> {
    /// The move's target (none for a point).
    pub(crate) target: Option<&'world Quarry>,
    /// The move's point.
    pub(crate) point: Position,
    /// The owner.
    pub(crate) player: PlayerId,
    /// What the hunter carries, and what its `MoveTo`s' constructors read of it.
    pub(crate) walks: Walks<'world>,
    /// The target is a drop site of the hunter's master.
    pub(crate) drops_here: bool,
    /// The pasture the unit is on (`mMyPastureId`).
    pub(crate) my_pasture: Option<PastureId>,
    /// The gap to a drop site target.
    pub(crate) gap: Distance,
}

impl HuntAction {
    /// The constructor (`FUN_140ac2dd0`), the hunt command's (`FUN_1412151f0`) and the factory's
    /// (`FUN_140aca4b0` case 0x6e with a target): a Hunt in state 0 on `task` (the base
    /// constructor `FUN_140a73e90` with 1: no second target, no sub-action, the timer 0), no drop
    /// site for livestock (`+0x70` −1) and the lead [`Lead::NEW`]. It holds `target`
    /// (`FUN_140a7a230`) unless it is past [`LAST_TAKEN`]; the take of a held target (slot 0x178)
    /// is the caller's. Only a held target gives the point (`+0x38`..`+0x40`) its place and the
    /// target type (`+0x68`) its master's class; else both stay −1, and the install's start
    /// searches ([`Unit::hunt_begin_search`]).
    ///
    /// The routine also takes a null target, which leaves the point and the target type −1 as
    /// one past state 6 does. No caller passes one: the hunt command makes the Hunt only on an
    /// object the world holds, and the factory calls this constructor only with a target; without
    /// one it calls another.
    ///
    /// [`Unit::hunt_begin_search`]: crate::objects::unit::Unit::hunt_begin_search
    #[must_use]
    pub fn new(task: HuntTask, target: &Quarry) -> Self {
        let (point, target_class, target) = if target.state <= LAST_TAKEN {
            let id = HuntTargetId::new(target.id);
            (target.position, target.class.or_none(), Some(id))
        } else {
            (Position::from([-1.0; 3]), None, None)
        };
        Self {
            target,
            state: HuntState::New,
            point,
            target_class,
            task,
            drop_site: None,
            carry_to: None,
            timer: Seconds::ZERO,
            lead: Lead::NEW,
        }
    }

    /// The install's start (slot 5 with 1, `0x140ac49b0`) of a Hunt that holds `target` (the
    /// constructor's, [`HuntAction::new`]): a target that holds something (`+0x310` above 0)
    /// gives the hunter the task's load ([`load_for_task`] on `held`); one holding nothing changes
    /// no load. A loaded hunter sent to one of its own drop sites carries the load there
    /// (`set_state` 8); else `set_state(4)` ([`HuntAction::chase`]): a target of another, unallied
    /// player (or of class 49) that is not dead yet is attacked first (`set_state` 12). How its
    /// Attack's `MoveTo` started (a walk to a drop site is under way). A Hunt whose constructor
    /// held no target does not start here: the slot then only searches (`set_state(3)`), which
    /// reads the hunter's step ([`Unit::hunt_begin_search`]).
    ///
    /// [`Unit::hunt_begin_search`]: crate::objects::unit::Unit::hunt_begin_search
    ///
    /// # Errors
    ///
    /// What the walks and the Attack do not model.
    pub(crate) fn start(
        &mut self,
        start: HuntStart<'_>,
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
        layer: BeginLayer<'_>,
        hunt_tuning: &HuntTuning,
    ) -> GenieResult<MoveStart> {
        let BeginLayer {
            roles,
            attack_tuning,
            move_to_tuning,
        } = layer;
        let HuntStart {
            target,
            player,
            player_kind,
            settings,
            held,
            walks,
        } = start;
        if Amount::ZERO < target.amount {
            *held = load_for_task(*held, self.task.resource, settings, player_kind);
        }
        let walks = Walks {
            held: *held,
            ..walks
        };
        // an own target among the hunter's master's drop sites (`+0x7a0`, FUN_14109d4f0; a
        // building annex's main building) takes a load there: it becomes the second target
        // (FUN_140a7a190), the target goes (slot 0x68 with none; the caller takes and releases
        // it), the point and `+0x68` clear, `set_state(8)`; a hunter carrying nothing does not
        // get there
        if target.player == player && target.drop_site && Amount::ZERO < held.amount {
            self.set_target(None);
            self.point = Position::from([-1.0; 3]);
            self.target_class = None;
            self.carry_to = Some(DropSiteId::new(target.id));
            self.set_drop(target.position, attack.gap, mover, &walks, move_to_tuning)?;
            return Ok(MoveStart::UnderWay);
        }
        let chase = HuntChase {
            target,
            player,
            walks: &walks,
        };
        self.chase(
            chase,
            attack,
            arms,
            mover,
            BeginLayer {
                roles,
                attack_tuning,
                move_to_tuning,
            },
            hunt_tuning,
        )
    }

    /// `set_state(4)` (`0x140ac60e0` case 4): the sub-actions go (an Attack's `MoveTo` stops the
    /// unit) and the timer clears. A hunt that holds no target (`+0x20` null: one past
    /// [`LAST_TAKEN`] that slot 0x68 left none) fails (`set_state(13)`). Else the target's place
    /// becomes the point. A dead target, or the hunter's own
    /// player's livestock, is walked to: a quiet `MoveTo` within the task's range, or
    /// `HuntTuning::own_livestock_range` to the livestock (`FUN_140aa0df0`; its start `0x140aa2100`: a unit
    /// within 0.01 of the target's footprint is there already). A target not dead yet of another,
    /// unallied player (or of class 49) is attacked first: `set_state(12)` (case 0xc) with an
    /// Attack of the master's ranges. A live allied one fails the hunt (`set_state` 13). How the
    /// Attack's `MoveTo` started: a walk and a failed hunt are under way.
    pub(crate) fn chase(
        &mut self,
        chase: HuntChase<'_>,
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
        layer: BeginLayer<'_>,
        hunt_tuning: &HuntTuning,
    ) -> GenieResult<MoveStart> {
        let BeginLayer {
            roles,
            attack_tuning,
            move_to_tuning,
        } = layer;
        let HuntChase {
            target,
            player,
            walks,
        } = chase;
        self.tear_down(Some(mover))?;
        self.state = HuntState::Walk { move_to: None };
        self.timer = Seconds::ZERO;
        if self.target.is_none() {
            // `+0x20` null: LAB_140ac679d, set_state(13), whose sprite is the caller's
            self.state = HuntState::Failed;
            return Ok(MoveStart::UnderWay);
        }
        self.point = target.position;
        let own_livestock =
            roles.of_class(target.class).contains(ClassRoles::LIVESTOCK) && target.player == player;
        if ObjectState::Alive < target.state || own_livestock {
            let range = if ObjectState::Alive < target.state {
                self.task.range
            } else {
                Distance::new(hunt_tuning.own_livestock_range)
            };
            let to = WalkTo {
                id: Some(target.id),
                at: target.position,
                gap: attack.gap,
                state: Some(target.state),
            };
            let move_to = self.move_to_object(to, range, mover, walks, move_to_tuning)?;
            self.state = HuntState::Walk { move_to };
            return Ok(MoveStart::UnderWay);
        }
        if !roles.of_class(target.class).contains(ClassRoles::FARM) && target.allied {
            // LAB_140ac679d: set_state(13): the timer clears; the unit's sprite (slot 0x68 with
            // FUN_140ac5210's) is the caller's
            self.state = HuntState::Failed;
            self.timer = Seconds::ZERO;
            return Ok(MoveStart::UnderWay);
        }
        self.state = HuntState::Attack { attack: None };
        let reads = walks.live(mover);
        let (attack, move_start) = Attack::start(
            target.id,
            attack,
            arms,
            mover,
            &reads,
            BeginLayer {
                roles,
                attack_tuning,
                move_to_tuning,
            },
        )?;
        self.state = HuntState::Attack {
            attack: Some(attack),
        };
        Ok(move_start)
    }

    /// Slot 9 (`0x140ac97c0`, slot 0x48): the new target of the work command, the hunt command or
    /// the hunt's own searches, for the hunt it keeps; slot 9 answers 1 whatever it did.
    ///
    /// - Without a target, or on a building under its Unpack ([`Quarry::unpacking`]), the hunt
    ///   walks to the point: the point (`+0x38`..`+0x40`) is the slot's, the target goes (slot
    ///   0x68 with none, `FUN_140ac7980`) and `set_state(15)` walks there
    ///   ([`HuntAction::set_point_walk`]). The target type (`+0x68`) and the second target stay.
    /// - Its own player's drop site (`drops_here`) while the hunter carries something (`+0x310`
    ///   above 0: `COMISS` and `JBE` at `0x140ac9925` send a NaN to the new target) takes the load
    ///   there: the target goes, the timer and the point clear, the drop site becomes the second
    ///   target (`FUN_140a7a190`), its class the target type, and `set_state(8)` walks there.
    /// - The target it holds (`+0x20`) changes nothing ([`Retargeted::Kept`]).
    /// - Any other target becomes its target (slot 0x68: none past [`LAST_TAKEN`],
    ///   `FUN_140a7a230`), its class the target type; the second target goes (`FUN_140a7a190`
    ///   with none) and `set_state(4)` chases the target ([`HuntAction::chase`]), which fails
    ///   without one.
    ///
    /// The caller lets go and takes the targets that changed ([`HuntAction::targets`] before and
    /// after), each pair in this order, as slot 0x68 (`FUN_140a7a230`) and `FUN_140a7a190` do:
    /// the drop site is taken whatever its state.
    ///
    /// # Errors
    ///
    /// What the walks and the Attack do not model.
    pub(crate) fn retarget(
        &mut self,
        retarget: HuntRetarget<'_>,
        arms: Arms,
        mover: &mut Mover,
        layer: BeginLayer<'_>,
        hunt_tuning: &HuntTuning,
    ) -> GenieResult<Retargeted> {
        let HuntRetarget {
            target,
            point,
            player,
            walks,
        } = retarget;
        let Some(&NewTarget {
            quarry: ref target,
            ref engagement,
            drops_here,
        }) = target.filter(|new| !new.quarry.unpacking())
        else {
            // 0x140ac9a95: the slot's point, slot 0x68 with none, set_state(15)
            self.point = point;
            self.set_target(None);
            self.set_point_walk(
                |move_to| HuntState::WalkPoint { move_to },
                mover,
                &walks,
                layer.move_to_tuning,
            )?;
            return Ok(Retargeted::Started(MoveStart::UnderWay));
        };
        if target.player == player && drops_here && Amount::ZERO < walks.held.amount {
            self.set_target(None);
            self.timer = Seconds::ZERO;
            self.point = Position::from([-1.0; 3]);
            self.carry_to = Some(DropSiteId::new(target.id));
            self.target_class = target.class.or_none();
            self.set_drop(
                target.position,
                engagement.gap,
                mover,
                &walks,
                layer.move_to_tuning,
            )?;
            return Ok(Retargeted::Started(MoveStart::UnderWay));
        }
        // the target pointer (`+0x20`), which FUN_140a7a230 keeps with the id
        if self.target.map(HuntTargetId::id) == Some(target.id) {
            return Ok(Retargeted::Kept);
        }
        self.set_target((target.state <= LAST_TAKEN).then(|| HuntTargetId::new(target.id)));
        self.target_class = target.class.or_none();
        self.carry_to = None;
        let chase = HuntChase {
            target,
            player,
            walks: &walks,
        };
        let move_start = self.chase(chase, engagement, arms, mover, layer, hunt_tuning)?;
        Ok(if self.state == HuntState::Failed {
            Retargeted::Failed
        } else {
            Retargeted::Started(move_start)
        })
    }

    /// Slot 0x40 (`FUN_140ac52c0`): the hunt takes a move's `target` (None for a point) and
    /// `point`; taken when it does. A point with a task of a work range it does not take. A unit
    /// on a pasture (`mMyPastureId` +0x794) of another target would leave it (`FUN_141189ca0`),
    /// not modelled. Its own player's drop site (`drops_here`) while it carries something
    /// (`held`) takes the load there: unless it walks there already (state 8), the target goes
    /// (slot 0x68 with none; the caller releases it), the point clears, `+0x68` takes the drop
    /// site's class, which becomes the second target (`FUN_140a7a190`; the caller takes it) and
    /// `set_state(8)`. Any other target, or a point: the point is the move's, `+0x68` −1, the
    /// target goes and `set_state(15)`: a quiet `MoveTo` to the point within the task's range
    /// (`FUN_140aa0d30`).
    pub fn take_move(
        &mut self,
        take: HuntMove<'_>,
        mover: &mut Mover,
        move_to_tuning: &MoveToTuning,
    ) -> GenieResult<Answer> {
        let HuntMove {
            target,
            point,
            player,
            walks,
            drops_here,
            my_pasture,
            gap,
        } = take;
        let held = walks.held.amount;
        if target.is_none() && self.task.range != Distance::ZERO {
            return Ok(Answer::NotTaken);
        }
        if my_pasture.is_some()
            && target.is_none_or(|quarry| Some(quarry.id) != my_pasture.map(PastureId::id))
        {
            return Err(GenieError::unsupported(
                "a hunter leaving its pasture (FUN_141189ca0)",
            ));
        }
        if let Some(quarry) =
            target.filter(|quarry| quarry.player == player && drops_here && Amount::ZERO < held)
        {
            if Some(quarry.id) == self.carry_to.map(DropSiteId::id)
                && matches!(self.state, HuntState::Drop { .. })
            {
                return Ok(Answer::Taken);
            }
            self.set_target(None);
            self.point = Position::from([-1.0; 3]);
            self.target_class = Some(quarry.class);
            self.carry_to = Some(DropSiteId::new(quarry.id));
            self.set_drop(quarry.position, gap, mover, &walks, move_to_tuning)?;
            return Ok(Answer::Taken);
        }
        self.point = point;
        self.target_class = None;
        self.set_target(None);
        self.set_point_walk(
            |move_to| HuntState::WalkPoint { move_to },
            mover,
            &walks,
            move_to_tuning,
        )?;
        Ok(Answer::Taken)
    }

    /// `set_state(11)` after a drop-off that found no next target: the walk to the hunt's point
    /// ([`HuntAction::set_point_walk`]), whose end searches again.
    ///
    /// # Errors
    ///
    /// A unit without speed.
    pub fn search_walk(
        &mut self,
        mover: &mut Mover,
        walks: &Walks<'_>,
        move_to_tuning: &MoveToTuning,
    ) -> GenieResult<()> {
        self.set_point_walk(
            |move_to| HuntState::WalkSearch { move_to },
            mover,
            walks,
            move_to_tuning,
        )
    }

    /// `set_state` 11 and 15 (`0x140ac60e0` case 0xb), the walk `walk` makes: the sub-actions go,
    /// the timer clears and a quiet `MoveTo` to the point within the task's range starts
    /// ([`HuntAction::move_to_point`]).
    pub(super) fn set_point_walk(
        &mut self,
        walk: fn(Option<MoveToAction>) -> HuntState,
        mover: &mut Mover,
        walks: &Walks<'_>,
        move_to_tuning: &MoveToTuning,
    ) -> GenieResult<()> {
        self.tear_down(Some(mover))?;
        self.state = walk(None);
        self.timer = Seconds::ZERO;
        let move_to = self.move_to_point(mover, walks, move_to_tuning)?;
        self.state = walk(move_to);
        Ok(())
    }

    /// State 4's wait after its `MoveTo` failed (`0x140ac7a00` case 3, its timer below 0): the
    /// timer runs by `dt`; at its end it clears and the hunt walks to its point (`set_state(11)`,
    /// the target kept), whose end searches.
    ///
    /// # Errors
    ///
    /// A unit that cannot move.
    pub fn wait_then_walk(
        &mut self,
        dt: Seconds,
        mover: &mut Mover,
        walks: &Walks<'_>,
        move_to_tuning: &MoveToTuning,
    ) -> GenieResult<()> {
        self.timer += dt;
        if self.timer < Seconds::ZERO {
            return Ok(());
        }
        self.timer = Seconds::ZERO;
        self.set_point_walk(
            |move_to| HuntState::WalkSearch { move_to },
            mover,
            walks,
            move_to_tuning,
        )
    }

    /// State 15's and 11's update (`0x140ac7a00` cases 0xe and 0xa) of `unit`'s hunt on the step,
    /// the hunt copied out of its list: the `MoveTo` to the point walks (the units in its way go
    /// to the unit AI at once, [`HuntAction::update_list`]); the motion step giving up turns it
    /// to its wait or its failure (`who`); a waiting one asks again after its wait (its case 2,
    /// its update returns 0). Once it ended (its update returns 1, 2 or 5) or failed (3, quiet: no
    /// notify), the `set_state` that follows tears it down (`FUN_140a98620`; its destructor stops
    /// the unit, slot 0x5d0): a human player's hunt in state 11 searches (`set_state(3)`: the
    /// state here, the rest the caller's), one in state 15 is done (`set_state(2)`: the target
    /// goes, the point clears and the timer; the unit takes its standing sprite and the second
    /// target goes, the caller's). A computer player's ended walk in state 15 (`set_state` 1) or
    /// failed one (`set_state` 13) is not modelled; a point walk has no target to lose (code 4).
    ///
    /// # Errors
    ///
    /// What the point walk does not model, and the walk's.
    pub(crate) fn update_point_walk(
        &mut self,
        step: &mut Step<'_>,
        unit: &mut Unit,
        me: Me,
        who: PointWalker,
    ) -> GenieResult<PointWalk> {
        let gives_up = GiveUp {
            patience: if !who.player_kind.is_computer() && who.villager {
                Patience::WaitsOnce
            } else {
                Patience::Fails
            },
            stance: who.stance,
        };
        let ended = match self.update_list(step, unit, me, gives_up)? {
            SubCode::UnderWay => return Ok(PointWalk::Walking),
            SubCode::Done | SubCode::TargetGone => true,
            SubCode::Failed => false,
        };
        if who.player_kind.is_computer() {
            return Err(GenieError::unsupported(if ended {
                "a computer player's hunt after its walk (set_state 1)"
            } else {
                "a computer player's hunt whose walk to a point failed (set_state 13)"
            }));
        }
        self.tear_down(step.world.parts.mover_mut(unit.id))?;
        if matches!(self.state, HuntState::WalkSearch { .. }) {
            self.state = HuntState::Search;
            return Ok(PointWalk::Search);
        }
        let [target, second] = self.set_done();
        Ok(PointWalk::Done { target, second })
    }

    /// `set_state(2)` (`0x140ac60e0` case 2): the hunt is done. The unit takes the sprite it
    /// stands with (slot 0x68 with `FUN_140ac5210`'s, the caller's), the target and the second
    /// target go (slot 0x68 and `FUN_140a7a190` with none: returned in that order for the caller
    /// to release), the point clears and the timer.
    pub fn set_done(&mut self) -> [Option<ObjectId>; 2] {
        self.state = HuntState::Idle;
        let target = self.target.take();
        let second = self.carry_to.take();
        self.point = Position::from([-1.0; 3]);
        self.timer = Seconds::ZERO;
        [target.map(HuntTargetId::id), second.map(DropSiteId::id)]
    }
}
