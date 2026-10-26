//! What a unit's actions hold: their taker, its top action, target and gait.

use crate::checkpoint::TopAction;
use crate::construction::build::BUILD;
use crate::masters::Master;
use crate::turn::BUILDING_KIND;
use crate::turn::unit::{TAKEN_OUT, Unit};
use crate::unit_ai::GenieError;

impl Unit {
    /// The stand-in left in a list for a unit taken out of it ([`TAKEN_OUT`]).
    #[must_use]
    pub(crate) fn taken_out() -> Self {
        Self {
            id: TAKEN_OUT,
            ..Self::default()
        }
    }

    /// Whether a building's slot 0x178 or 0x180 (`FUN_1411dba70`, `FUN_1411d6020`) lists its taker
    /// among the attacking units (`+0xce0`, [`Unit::attacking_units`]): this object of `master` is a
    /// building (type 80) below state 2 whose master has a stack unit (`+0x986`), and the taker's
    /// player is not allied with its owner (`FUN_141663d90` with 1: `allied` false).
    pub(crate) fn lists_taker(&self, master: &Master, allied: bool) -> bool {
        master.kind == BUILDING_KIND
            && self.life.state < 2
            && master.building.is_some_and(|b| b.stack_unit != -1)
            && !allied
    }

    /// Slot 0x178 on this object of its player's live list, taken by the object `by` (`listed`,
    /// [`Unit::lists_taker`]). A building's (`FUN_1411dba70`) first lists the taker among its attacking
    /// units (`+0xce0`, [`Unit::attacking_units`]; once per take). Then the base take
    /// (`FUN_14119d3e0`): its worker count (`+0x644`) rises, to at most 550, and a live object flagged
    /// to sleep drops the flags (a sleeping one's wake is [`crate::turn::Turns::take`]'s). The
    /// building's wake from semi-asleep (`+0xc98`) after it never runs here: only the building's
    /// load (`FUN_1411df160` -> `FUN_1411ddd20`) sets that flag, from the save's
    /// `temp_semi_asleep`, which no saved building of the 158041 recordings has (1473 headers
    /// read); the slice does not hold it.
    pub(crate) fn taken(&mut self, listed: bool, by: i32) {
        if listed {
            self.attacking_units.push(by);
        }
        if self.workers < 550 {
            self.workers += 1;
        }
        self.life.goto_sleep = false;
    }

    /// The target of its hunt, which it attacks or gathers from.
    pub(in crate::turn) fn attack_target(&self) -> Option<i32> {
        self.hunt
            .map(|h| h.target)
            .or_else(|| self.gather.map(|g| g.target).filter(|&t| t != -1))
            .or_else(|| self.attack.map(|a| a.target))
            .or_else(|| {
                self.missile
                    .as_ref()
                    .and_then(|m| m.action.as_ref())
                    .map(|a| a.target)
                    .filter(|&t| t != -1)
            })
    }

    /// `why` with what this object was doing in its context: its id and master, its top action
    /// (type and state) and its unit AI's order, action and target.
    #[must_use]
    pub fn in_context(&self, why: GenieError) -> GenieError {
        let mut why = why
            .with("object", self.id)
            .with("master", self.life.master_id);
        if let Some(a) = self.top_action() {
            why = why.with("action", format_args!("{} state {}", a.kind, a.state));
        }
        if let Some(ai) = &self.ai {
            why = why.with(
                "unit AI",
                format_args!(
                    "order {} action {} target {}",
                    ai.current_order, ai.current_action, ai.current_target
                ),
            );
        }
        why
    }

    /// What the walk reads of it when it meets it ([`crate::spatial::Gait`]).
    #[must_use]
    pub fn gait(&self) -> Option<crate::spatial::Gait> {
        let m = self.mover.as_ref()?;
        Some(crate::spatial::Gait {
            moving: m
                .velocity
                .as_ref()
                .is_some_and(|v| 0.0 < crate::math::fmath::length(v.velocity)),
            waits: m.waiting != 0 && !self.top_action().is_some_and(|a| matches!(a.state, 6 | 7)),
            last_move: m.last_move,
            pause: m.paths.first().map(|l| l.pause_time),
            human_order: self.last_human_order,
            path_made: m
                .paths
                .first()
                .filter(|l| !l.points.is_empty())
                .map(|l| l.created),
            group: Some(self.group.id).filter(|&g| g != -1),
        })
    }

    /// Its top action's slots 0x190 and 0x288 (`FUN_14106d090`, `FUN_141082dc0`): the action has
    /// neither a target nor a point (`+0x30`, `+0x38` −1), or there is none. An Artifact holds
    /// them while it walks a move ([`crate::artifact::Artifact::idle`]); a `MoveTo` always holds
    /// its point.
    ///
    /// # Errors
    ///
    /// A top action the slice does not model.
    pub fn top_action_idle(&self) -> Result<bool, GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "slots 0x190 and 0x288 of an action the slice does not model",
            ));
        }
        match self.top_action_kind() {
            None => Ok(true),
            Some(crate::artifact::ARTIFACT_TYPE) => Ok(self.artifact.is_some_and(|a| a.idle())),
            // a Make has no target, but its point (`+0x38`) is the base constructor's 0.0
            Some(crate::move_to::MOVE_TO | crate::gathering::make::MAKE) => Ok(false),
            Some(_) => Err(GenieError::unsupported(
                "slots 0x190 and 0x288 of an action the slice does not model",
            )),
        }
    }

    /// The type of its top action, if it has one.
    #[must_use]
    pub fn top_action_kind(&self) -> Option<i16> {
        self.top_action().map(|a| a.kind)
    }

    /// Its top action as the slice models it, type and state.
    #[must_use]
    pub fn top_action(&self) -> Option<TopAction> {
        let action = |kind, state| TopAction { kind, state };
        None.or_else(|| {
            self.artifact
                .map(|a| action(crate::artifact::ARTIFACT_TYPE, a.state))
        })
        .or_else(|| self.build.map(|b| action(BUILD, b.state)))
        .or_else(|| {
            self.repair
                .map(|r| action(crate::construction::repair::REPAIR, r.state))
        })
        .or_else(|| {
            self.move_to
                .map(|m| action(crate::move_to::MOVE_TO, m.state.number()))
        })
        .or_else(|| self.fly.map(|f| action(crate::fly::FLY, f.state)))
        .or_else(|| {
            self.hunt
                .map(|h| action(crate::gathering::hunt::HUNT, h.state))
        })
        .or_else(|| {
            self.gather
                .map(|g| action(crate::gathering::gather::GATHER, g.state))
        })
        .or_else(|| {
            self.attack
                .map(|a| action(crate::combat::attack::ATTACK, a.state))
        })
        .or_else(|| {
            self.explore
                .map(|e| action(crate::explore::EXPLORE, e.state))
        })
        .or_else(|| self.enter.map(|e| action(crate::garrison::ENTER, e.state)))
        .or_else(|| {
            self.transport
                .map(|t| action(crate::garrison::transport::TRANSPORT, t.state))
        })
        .or_else(|| {
            self.missile
                .as_ref()
                .and_then(|m| m.action.as_ref())
                .map(|a| action(crate::combat::missile::MISSILE, a.state))
        })
        .or_else(|| {
            self.make
                .map(|m| action(crate::gathering::make::MAKE, m.state))
        })
        .or_else(|| {
            self.wait_solidify
                .map(|w| action(crate::construction::wait_solidify::WAIT_SOLIDIFY, w.state))
        })
    }

    /// The target of its top action ([`Unit::top_action`]), −1 for an action without one (an
    /// Artifact not moving to an object, a Fly, an Explore, a wait for a foundation, which keeps
    /// the foundation at `+0x68`); None without an action the slice holds.
    #[must_use]
    pub fn top_target(&self) -> Option<i32> {
        if let Some(a) = self.artifact {
            return Some(a.target);
        }
        None.or_else(|| self.build.map(|b| b.target))
            .or_else(|| self.repair.map(|r| r.target))
            .or_else(|| self.move_to.map(|m| m.target))
            .or_else(|| self.fly.map(|_| -1))
            .or_else(|| self.hunt.map(|h| h.target))
            .or_else(|| self.gather.map(|g| g.target))
            .or_else(|| self.attack.map(|a| a.target))
            .or_else(|| self.explore.map(|_| -1))
            .or_else(|| self.enter.map(|e| e.target))
            .or_else(|| self.transport.map(|_| -1))
            .or_else(|| {
                self.missile
                    .as_ref()
                    .and_then(|m| m.action.as_ref())
                    .map(|a| a.target)
            })
            .or_else(|| self.make.map(|_| -1))
            .or_else(|| self.wait_solidify.map(|_| -1))
    }

    /// The second target of its top action (`+0x34`, `FUN_140a78430`), −1 for none: a Hunt's
    /// or a Gather's drop site, a missile's shooter; the slice's other actions hold none (an import checks the
    /// save's). None without an action the slice holds.
    #[must_use]
    pub fn top_second_target(&self) -> Option<i32> {
        match self.top_action_kind()? {
            crate::gathering::hunt::HUNT => self.hunt.map(|h| h.carry_to),
            crate::gathering::gather::GATHER => self.gather.map(|g| g.carry_to),
            crate::combat::missile::MISSILE => self
                .missile
                .as_ref()
                .and_then(|m| m.action.as_ref())
                .map(|a| a.fired_from),
            crate::combat::attack::ATTACK => self.attack.map(|a| a.target2),
            _ => Some(-1),
        }
    }

    /// Slot 0x2d0 with 0 (`FUN_14106d170`): whether its top action may be interrupted (the group
    /// test first; the slice holds no groups). None may; type `0x7e` never; by state, 0, 1, 3,
    /// 0xd and 0xe may, 2 unless an Attack (9); in any other state only an Artifact (`0x6b`)
    /// without a target or a point (`+0x30`, `+0x38` = −1). A trade (`0x6f`) in state 4 is not
    /// modelled, nor is an action the slice does not hold.
    pub(crate) fn interruptible(&self) -> Result<bool, GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "slot 0x2d0 of an action the slice does not model",
            ));
        }
        let Some(TopAction { kind, state }) = self.top_action() else {
            return Ok(true);
        };
        Ok(match (kind, state) {
            (0x7e, _) => false,
            (0x6f, 4) => return Err(GenieError::unsupported("slot 0x2d0 of a trade in state 4")),
            (_, 0 | 1 | 3 | 0xd | 0xe) => true,
            (_, 2) => kind != crate::combat::attack::ATTACK,
            (crate::artifact::ARTIFACT_TYPE, _) => self
                .artifact
                .is_some_and(|a| a.goal.is_none_or(|g| g[0] == -1.0)),
            _ => false,
        })
    }
}
