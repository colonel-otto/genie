//! A builder's Build action (`TRIBE_Action_Build`, type 101, vtable `0x142ed6628`) up to the
//! builder's walk: the action starts in state 4 with a `MoveTo` sub-action (`RGE_Action_Move_To`,
//! type 1) toward the foundation; a builder already at the foundation works in its first update
//! (state 6). A walking builder's `MoveTo` runs the motion step ([`crate::movement`]).
//!
//! A builder at a foundation faces it and joins its work (state 7): at a foundation no one started
//! it first checks the footprint, and a builder standing on it walks to a free tile beside it
//! (state 223) and comes back (state 4). The work adds to the foundation's construction each
//! update ([`Construction::work`]); the first work starts it: the foundation gets its obstruction
//! record. The work that reaches the build time completes the building ([`Construction::complete`])
//! and ends the builder's work (state 224); a builder whose unit AI holds queued orders then lets
//! the Build end (state 1) and the AI runs its queue.

use crate::lifecycle::Lifecycle;
use crate::masters::Master;
use crate::move_to::{Goal, start_walk, walk_on};
pub use crate::move_to::{MoveTo, Walk};
use crate::movement::footprint_gap;
use crate::pathing::Pathing;
use crate::spatial::{Obstruction, Spatial};
use crate::unit_ai::{UnitAi, Unsupported};

/// The Build action's type, which word 9 adds.
pub const BUILD: i16 = 101;

/// The gap below which a `MoveTo` has arrived (`DAT_142eb99cc`).
const ARRIVED: f32 = crate::image::build::ARRIVED;
/// The villager's class (4, `-0xea8` encoded): Build's walk off the footprint returns at once.
const VILLAGER: i16 = 4;

/// The Build action of a builder (`TRIBE_Action_Build`, type 101).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildAction {
    /// The foundation's id.
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// Its `MoveTo` sub-action's state.
    pub move_to: Option<MoveTo>,
    /// The point the `MoveTo` walks to (its `+0x38`..`+0x40`), when not to the foundation.
    pub goal: Option<[f32; 3]>,
    /// The task it builds with.
    pub task: BuildTask,
    /// `+0x68`, `added_to_num_builders`: the builder counts among the foundation's builders.
    pub added: bool,
}

/// What a builder's Build task gives its work: the task's work range (`+0x30`; its `MoveTo` to the
/// foundation stops this near) and work value (`+0x28`, `work_val1`), and the builder's work rate:
/// its master's `work_rate` (`+0x798`) plus its own `mWorkRatePowerup` (`+0xa30`, slot 0x6e8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildTask {
    /// The task's work range.
    pub range: f32,
    /// The task's work value.
    pub work_value: f32,
    /// The builder's work rate: its master's `work_rate` (`+0x798`) plus its `mWorkRatePowerup`
    /// (`+0xa30`).
    pub work_rate: f32,
}

/// A building under construction (`TRIBE_Building_Object`), with what its master says of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Construction {
    /// `+0xc50`, `build_pts`: the seconds of work it has had.
    pub progress: f32,
    /// `+0xc60`, `desolid_flag`: no builder has started it; it has no obstruction record.
    pub desolid: bool,
    /// `+0x9e8`, `num_builders`.
    pub builders: u8,
    /// Its master's hit points (`+0x530`) and build time (`+0x898`, the first train location's
    /// train time; 0 without one).
    pub hit_points: i16,
    /// Its master's build time (the first train location's train time; 0 without one).
    pub build_time: i16,
    /// The record its start adds: its master's obstruction kind and class.
    pub record: Obstruction,
    /// `+0xc61`, `snow_flag`: it was completed on snow (its decal is not held).
    pub snow: bool,
    /// `+0x788`, `sprite_override_id`, is not −1.
    pub sprite_override: bool,
}

/// What a builder's work reaches of its foundation.
#[derive(Debug)]
pub struct Site<'a> {
    /// The foundation.
    pub id: i32,
    /// Its life cycle.
    pub life: &'a mut Lifecycle,
    /// Its construction.
    pub construction: &'a mut Construction,
    /// The foundation's movement state (its last move time), if the slice holds it.
    pub mover: Option<&'a mut crate::movement::Mover>,
    /// What its completion reads ([`Construction::complete`]).
    pub completion: Completion<'a>,
}

/// What a building's completion reads besides its construction.
#[derive(Clone, Copy, Debug)]
pub struct Completion<'a> {
    /// The building's master.
    pub master: Option<&'a Master>,
    /// Another object is its annex (its `+0xbc0` list).
    pub annexed: bool,
    /// Some player's attribute 204 is not 0 (`FUN_14152bcb0`).
    pub attribute_204: bool,
    /// The game's setting at `+0x680` ([`crate::turn::Turns::mode_680`]).
    pub mode_680: i32,
}

/// What a builder's work did to its foundation ([`Construction::work`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Worked {
    /// The construction goes on.
    Going,
    /// The foundation was no longer under construction (state not 0, or no hit points).
    Done,
    /// This work completed it.
    Completed(Built),
}

/// What a completion gives its owner ([`Construction::complete`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Built {
    /// The attributes it gains, (attribute, amount), in the order the game adds them.
    pub attributes: Vec<(i16, f32)>,
    /// The master's technology (`+0x98c`, the building's `tech`), which the completion researches
    /// last (`FUN_1413cdcd0` without a building): the caller's.
    pub tech: Option<i16>,
}

/// What a builder brings to its Build action's update besides its walk.
#[derive(Debug)]
pub struct Builder<'a> {
    /// The builder's class (`+0x0a`, decoded).
    pub class: i16,
    /// What it carries (`+0x310`).
    pub held: f32,
    /// Its unit AI, which a finished Build hands its queued orders to.
    pub ai: Option<&'a mut UnitAi>,
}

/// What a Build action's update leaves to its unit and player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildStep {
    /// The update returned 1: the action list removes the action, whose destructor
    /// (`FUN_140aadbf0`) releases the foundation (slot 0x180).
    pub done: bool,
    /// The work completed the building: what its owner gains.
    pub completed: Option<Built>,
    /// The work ended without more work: the action let its foundation go (slot 0x68 with none,
    /// `FUN_140ab0300` -> `FUN_140a7a230`: the foundation's slot 0x180) and stays in state 3,
    /// whose `set_state` posted notify 0x202 (602) to the builder's unit AI: the caller's.
    pub released: bool,
    /// The work ended and the builder looks for more work around the building (`FUN_1415cc4e0`,
    /// [`crate::gather_search::more_work`]): the caller's, which works what it finds, else lets
    /// the foundation go ([`BuildAction::no_more_work`], then as [`BuildStep::released`]).
    pub search: bool,
}

/// A second builder works at a third of the rate of the first (`DAT_142ed66b0`).
const EXTRA_BUILDER: f32 = crate::image::build::EXTRA_BUILDER;
/// The progress a started foundation holds before its first work (`0x2edbe6ff`, about 1e-10).
const STARTED: u32 = 0x2edb_e6ff;
/// The Wonder's master (`0x114`), whose construction rate the player's attribute raises at most to
/// `DAT_142ed66b4`.
const WONDER: i16 = 0x114;
const WONDER_RATE: f32 = crate::image::build::WONDER_RATE;
/// The wall class (27, `0x1b`): two walls beside a builder on a footprint's diagonal stop it.
const WALL: i16 = 27;

/// The Build action's states the slice models.
const MOVE: u8 = 4;
const WORK: u8 = 6;
const PLACE: u8 = 7;
/// 223 (`0xdf`): off the footprint, to a free tile beside it.
const CLEAR: u8 = 223;
/// 224 (`0xe0`): the work ended, the building completed.
const FINISHED: u8 = 224;
/// 1: the action is done.
const DONE: u8 = 1;
/// 3: the action waits without a target.
const IDLE: u8 = 3;
/// The masters and the class the end of the work treats apart: 621 and 1889 among the
/// foundations, class 39 among the builders, 1893 and 1897 (`id - 0x765 & !4 == 0`), the fish
/// trap 199 and farms (class 49, `FUN_141194b20`).
const MASTER_621: i16 = 621;
const MASTER_1889: i16 = 1889;
const CLASS_39: i16 = 39;
const FISH_TRAP: i16 = 199;
const FARM: i16 = 49;
/// The build order (`0x2be`), which the end of the work drops from the AI's queue once its
/// foundation is gone or built (`FUN_140c5dcd0`).
const BUILD_ORDER: i32 = crate::ai_kinds::ORDER_BUILD;

impl BuildAction {
    /// The action's start (slot 5, `0x140aadd80`): state 4 for a target, whose `set_state`
    /// (`0x140aae200`) creates the `MoveTo`; the `MoveTo` starts arrived if the footprints touch.
    #[must_use]
    pub fn start(target: i32, gap: f32, task: BuildTask) -> Self {
        Self {
            target,
            state: MOVE,
            move_to: Some(if gap > ARRIVED {
                MoveTo::Start
            } else {
                MoveTo::Arrived
            }),
            goal: None,
            task,
            added: false,
        }
    }

    /// One update (slot 6, `0x140ab0370`), which first updates the `MoveTo` (`0x140aa2a70`); a
    /// moving `MoveTo` calls the motion step (object slot 193). `site` is the foundation, when it
    /// is the builder's player's. Before its state's case it checks its foundation: one that is
    /// no longer under construction (a state above 0) ends the work (`set_state(224)`) unless the
    /// Build already ended (224, 4, 8); one that is gone, a dead one, master 621, a builder of
    /// class 39 and master 1889 of a human player are not modelled.
    ///
    /// # Panics
    ///
    /// Never: a builder clearing the footprint (state 223) always has the point its `set_state`
    /// gave it.
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        site: Option<Site<'_>>,
        builder: Builder<'_>,
    ) -> Result<BuildStep, Unsupported> {
        let mut step = BuildStep::default();
        // state 3 without a target: the update does nothing (0x140ab0370's default case)
        if self.state == IDLE && self.target == -1 {
            return Ok(step);
        }
        let computer = walk.plan.as_ref().is_some_and(|p| p.computer);
        let mut site = site;
        if let Some(site) = site.as_mut() {
            let master = site.completion.master;
            if !computer && master.is_some_and(|m| m.id == MASTER_1889) {
                return Err(Unsupported("a Build on master 1889"));
            }
            let state = site.life.state;
            if state != 0 && !matches!(self.state, FINISHED | MOVE | 8) {
                if master.is_some_and(|m| m.id == MASTER_621) || builder.class == CLASS_39 {
                    return Err(Unsupported(
                        "the end of a Build on master 621 or by class 39",
                    ));
                }
                if 3 <= state {
                    return Err(Unsupported("a Build whose foundation died"));
                }
                self.finish(&mut walk, Some(&mut *site.construction), builder.held)?;
            }
        } else if walk.world.get(self.target).is_none() {
            return Err(Unsupported("a Build whose foundation is gone"));
        }
        if self.state == FINISHED {
            match self.after_work(&walk, site.as_ref(), computer, builder)? {
                Some(true) => step.done = true,
                Some(false) => step.search = true,
                None => step.released = true,
            }
            return Ok(step);
        }
        let mut walk = walk;
        let held = builder.held;
        let r = match (self.state, self.move_to) {
            (4, Some(MoveTo::Arrived)) => {
                self.state = 6;
                self.move_to = None;
                Ok(())
            }
            (4, Some(MoveTo::Start)) => {
                self.move_to = Some(MoveTo::Moving);
                let range = self.task.range;
                self.move_to = Some(start_walk(walk, Goal::Target(self.target), range, false)?);
                Ok(())
            }
            (4 | CLEAR, Some(MoveTo::Moving)) => {
                self.move_to = Some(walk_on(walk)?);
                Ok(())
            }
            (4, None) => Err(Unsupported("a Build action in state 4 without its MoveTo")),
            (WORK, _) => {
                // case 5: FUN_141162db0 turns the builder to face its foundation
                let at = walk
                    .world
                    .get(walk.me.id)
                    .map_or(walk.me.position, |p| p.position);
                let to = walk
                    .world
                    .get_known(self.target)?
                    .ok_or(Unsupported("a builder's foundation without a place"))?
                    .position;
                let mover = walk
                    .mover
                    .as_deref_mut()
                    .ok_or(Unsupported("a builder without movement state"))?;
                let now = walk.plan.as_ref().map_or(0, |p| p.now);
                if !crate::attack::face_target(mover, at, to, now)? {
                    return Err(Unsupported(
                        "a builder that turns over time to face its foundation",
                    ));
                }
                self.place(&walk, site)
            }
            (PLACE, _) => self.work(walk, site, held).map(|c| step.completed = c),
            (CLEAR, Some(MoveTo::Start)) => {
                self.move_to = Some(MoveTo::Moving);
                let goal = self.goal.expect("set_state(223) sets the goal");
                self.move_to = Some(start_walk(walk, Goal::Point(goal), 0.0, false)?);
                Ok(())
            }
            // case 0xde: the MoveTo ended (its update in state 1 returns 1): a villager goes
            // back to the foundation (set_state 4)
            (CLEAR, Some(MoveTo::Arrived)) => self.back(&walk),
            (CLEAR, _) => Err(Unsupported(
                "a builder off its footprint without its MoveTo",
            )),
            _ => Err(Unsupported("a Build state not modelled")),
        };
        r.map(|()| step)
    }

    /// The prelude of every `set_state` (`0x140aae200`): a builder counted among the foundation's
    /// builders leaves them, and the sub-actions go (`FUN_140a98620`: a `MoveTo`'s teardown stops
    /// the unit, slot 0x5d0).
    fn leave(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<&mut Construction>,
    ) -> Result<(), Unsupported> {
        if self.added {
            if let Some(c) = site {
                c.builders = c.builders.wrapping_sub(1);
            }
            self.added = false;
        }
        if self.move_to.take().is_some()
            && let Some(m) = walk.mover.as_deref_mut()
        {
            m.stop()?;
        }
        self.goal = None;
        Ok(())
    }

    /// `set_state(224)` (`0x140aae200` case 0xe0): the work ended. A builder carrying a resource
    /// the finished building takes (`FUN_140fde180`, then `FUN_140aadc50`'s drop) is not
    /// modelled.
    fn finish(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<&mut Construction>,
        held: f32,
    ) -> Result<(), Unsupported> {
        if self.added && site.is_none() {
            return Err(Unsupported(
                "the end of a Build counted among another player's builders",
            ));
        }
        self.leave(walk, site)?;
        self.state = FINISHED;
        if 0.0 < held {
            return Err(Unsupported(
                "a builder carrying a resource when its building completes (FUN_140fde180)",
            ));
        }
        Ok(())
    }

    /// Case 0xdf of the update, the work ended (224): a farm, the fish trap and masters 1893 and
    /// 1897 are not modelled; the builder's unit AI drops its build orders on foundations gone or
    /// built (`FUN_140c5dcd0`); with orders left in its queue (`FUN_140c564a0`) it has no order
    /// and no action (`FUN_140c64b60`, `FUN_140c64b50` with −1) and the Build ends (`set_state(1)`,
    /// then the update returns 1: Some(true)). A builder without queued orders looks for more
    /// work around the building (`FUN_1415cc4e0`, Some(false): [`BuildStep::search`], the
    /// caller's). A computer player's builder is not modelled.
    fn after_work(
        &mut self,
        walk: &Walk<'_>,
        site: Option<&Site<'_>>,
        computer: bool,
        builder: Builder<'_>,
    ) -> Result<Option<bool>, Unsupported> {
        if computer {
            return Err(Unsupported("a computer player's builder after its work"));
        }
        let master = site.and_then(|s| s.completion.master).ok_or(Unsupported(
            "the end of a Build on a foundation the slice does not read",
        ))?;
        if master.class == FARM || master.id == FISH_TRAP || matches!(master.id, 1893 | 1897) {
            return Err(Unsupported(
                "the end of a Build on a farm, a fish trap or master 1893/1897",
            ));
        }
        let ai = builder
            .ai
            .ok_or(Unsupported("a builder without a unit AI after its work"))?;
        if ai.orders.iter().any(|o| o.order == BUILD_ORDER) {
            return Err(Unsupported(
                "a builder's queued build orders (FUN_140c5dcd0)",
            ));
        }
        if ai.orders.is_empty() {
            return Ok(Some(false));
        }
        ai.end_order_and_action()?;
        self.done(walk)?;
        Ok(Some(true))
    }

    /// The search for more work found nothing (`0x140ab0f00`): slot 0x68 with none lets the
    /// foundation go (the caller's) and `set_state(3)` waits.
    ///
    /// # Errors
    ///
    /// A Build with its `MoveTo` (`set_state`'s teardown is not modelled for it).
    pub const fn no_more_work(&mut self) -> Result<(), Unsupported> {
        if self.move_to.is_some() {
            return Err(Unsupported("a Build waiting with its MoveTo"));
        }
        self.target = -1;
        self.state = IDLE;
        Ok(())
    }

    /// `set_state(1)` (`0x140aae200` case 1): the builder takes its standing sprite (slot 0x68,
    /// not held for builders) and its owner's slot 0x380 hears of it (`FUN_141618d30`: only a
    /// computer player's AI listens).
    fn done(&mut self, walk: &Walk<'_>) -> Result<(), Unsupported> {
        if walk.mover.as_ref().is_some_and(|_| self.move_to.is_some()) {
            return Err(Unsupported("a Build ending with its MoveTo"));
        }
        self.state = DONE;
        Ok(())
    }

    /// `set_state(7)` (`0x140aae200` case 7) on a foundation no builder started: clear the
    /// footprint of what may be built over, find who stands on it, and send this builder off it
    /// if it stands there itself.
    fn place(&mut self, walk: &Walk<'_>, site: Option<Site<'_>>) -> Result<(), Unsupported> {
        let mut site = site.ok_or(Unsupported("a foundation of another player"))?;
        // every set_state first takes the builder out of the foundation's builders
        if self.added {
            site.construction.builders = site.construction.builders.wrapping_sub(1);
            self.added = false;
        }
        self.move_to = None;
        self.goal = None;
        self.state = PLACE;
        if !site.construction.desolid {
            return self.join(walk, &mut site);
        }
        let world = &*walk.world;
        let placed = world
            .get_known(self.target)?
            .ok_or(Unsupported("a foundation that is gone"))?;
        let site_ref = &mut site;
        let site = placed;
        if !world.footprint_clearing(self.target, site)?.is_empty() {
            return Err(Unsupported("a foundation clears objects off its footprint"));
        }
        if site.obstruction_type == 0 || site.obstruction_type == 12 {
            return Err(Unsupported("a foundation whose master does not obstruct"));
        }
        let occupants = world.footprint_occupants(self.target, site)?;
        if occupants.iter().any(|&id| id != walk.me.id) {
            return Err(Unsupported(
                "another unit on the footprint (its evacuation)",
            ));
        }
        if world.foreign_marks(site) {
            return Err(Unsupported("another player's marks on the footprint"));
        }
        if occupants.is_empty() {
            return self.join(walk, site_ref);
        }
        let [bx, by, _] = walk.me.position;
        let [sx, sy, _] = site.position;
        let inside = (bx - sx).abs() <= site.radius[0] && (by - sy).abs() <= site.radius[1];
        if !inside {
            return Err(Unsupported(
                "a builder waits for its footprint to clear (state 222)",
            ));
        }
        self.clear(walk)
    }

    /// `LAB_140aaf9f4`: the builder joins the work: unless walls of other players stand on both
    /// tiles beside it on the footprint's diagonal (`set_state` 2, not modelled), it counts among
    /// the foundation's builders. Its work sprite and sound are not held by the slice.
    fn join(&mut self, walk: &Walk<'_>, site: &mut Site<'_>) -> Result<(), Unsupported> {
        let placed = walk
            .world
            .get(self.target)
            .ok_or(Unsupported("a foundation that is gone"))?;
        let [bx, by] = [walk.me.position[0] as i32, walk.me.position[1] as i32];
        let [tx, mut ty] = [placed.position[0] as i32, placed.position[1] as i32];
        let [rx, ry] = placed.radius;
        if (rx as i32) as f32 == rx && (ry as i32) as f32 == ry {
            if by < ty {
                if tx < bx {
                    ty -= 1;
                }
            } else if ty < by && bx < tx {
                ty -= 1;
            }
        }
        let (dx, dy) = (bx - tx, by - ty);
        if dx.abs() == dy.abs() {
            let (a, b) = beside_diagonal(bx, by, dx, dy);
            let player = placed.player;
            let walled = |[x, y]: [i32; 2]| {
                walk.world
                    .object_on_tile(x, y, |o| o.class == WALL && o.player != player)
            };
            if walled(a) && walled(b) {
                return Err(Unsupported("walls beside a diagonal builder (set_state 2)"));
            }
        }
        if !self.added {
            site.construction.builders = site.construction.builders.wrapping_add(1);
            self.added = true;
        }
        Ok(())
    }

    /// The work (state 7, `0x140ab0370` case 6): the builder adds its rate times dt to the
    /// foundation's construction: its work rate times the task's work value, times the owner's
    /// construction rate (attribute 195) when that is above 0 (the Wonder's at most 1.2), each
    /// builder after the first adding a third ([`Construction::work`]). A finished building, a
    /// farm's reseeding and the instant-build cheat (`game+0xc14`) are not modelled.
    fn work(
        &mut self,
        mut walk: Walk<'_>,
        site: Option<Site<'_>>,
        held: f32,
    ) -> Result<Option<Built>, Unsupported> {
        let mut site = site.ok_or(Unsupported("a foundation of another player"))?;
        let plan = walk
            .plan
            .as_mut()
            .ok_or(Unsupported("work without the world's planners"))?;
        let mut rate = self.task.work_rate * self.task.work_value * walk.dt;
        let mut factor = plan.construction_rate;
        if site.life.master_id == WONDER && WONDER_RATE < factor {
            factor = WONDER_RATE;
        }
        if 0.0 < factor {
            rate *= factor;
        }
        let n = f32::from(site.construction.builders);
        if 1.0 < n {
            rate *= ((n - 1.0) * EXTRA_BUILDER + 1.0) / n;
        }
        let (pathing, now) = (&mut *plan.pathing, plan.now);
        let worked = Construction::work(&mut site, rate, walk.world, pathing, now)?;
        let completed = match worked {
            Worked::Going => return Ok(None),
            Worked::Done => None,
            Worked::Completed(built) => Some(built),
        };
        // a finished foundation with hit points ends the work (its `+0x278`, decoded)
        if 0.0 < site.life.hp {
            self.finish(&mut walk, Some(&mut *site.construction), held)?;
        } else {
            return Err(Unsupported(
                "work on a finished foundation without hit points",
            ));
        }
        Ok(completed)
    }

    /// `set_state(4)` after the walk off the footprint (`0x140aae200` case 4): the goal is the
    /// foundation again and a new `MoveTo` to it (`FUN_140aa0df0`) starts arrived if the footprints
    /// touch, as [`BuildAction::start`]'s. Only a villager returns at once (`0x140ab0370` case
    /// 0xde; any other builder in range would work, state 6).
    fn back(&mut self, walk: &Walk<'_>) -> Result<(), Unsupported> {
        let mover = walk
            .mover
            .as_deref()
            .ok_or(Unsupported("a builder without movement state"))?;
        if walk.world.get(walk.me.id).map(|p| p.class) != Some(VILLAGER) {
            return Err(Unsupported("a builder other than a villager returns"));
        }
        let site = walk
            .world
            .get_known(self.target)?
            .ok_or(Unsupported("a foundation that is gone"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(walk.me.position, ground, site.position, site.radius);
        *self = Self::start(self.target, gap, self.task);
        Ok(())
    }

    /// `set_state(223)`: a `MoveTo` to the free tile beside the footprint (`FUN_1411566b0`); it starts
    /// in state 3, as a `MoveTo` to a point is far from it (`FUN_140aa2100`).
    fn clear(&mut self, walk: &Walk<'_>) -> Result<(), Unsupported> {
        let mover = walk
            .mover
            .as_deref()
            .ok_or(Unsupported("a builder without movement state"))?;
        let site = walk.world.get(self.target).expect("place found it");
        let spot = walk
            .world
            .spot_beside(walk.me.position, mover.stats.restriction_mask, site)?
            .ok_or(Unsupported(
                "no free tile beside the footprint (set_state 2)",
            ))?;
        if mover.stats.speed == 0.0 {
            return Err(Unsupported("a builder that cannot move (MoveTo state 13)"));
        }
        self.state = CLEAR;
        self.move_to = Some(MoveTo::Start);
        self.goal = Some([spot[0], spot[1], walk.me.position[2]]);
        Ok(())
    }
}

/// The two tiles beside a builder on a footprint's diagonal (`LAB_140aafc09`'s arguments).
const fn beside_diagonal(bx: i32, by: i32, dx: i32, dy: i32) -> ([i32; 2], [i32; 2]) {
    if dx < 1 {
        if dx < 0 && dy < 0 {
            ([bx + 1, by], [bx, by + 1])
        } else if dx < 0 && 0 < dy {
            ([bx, by - 1], [bx + 1, by])
        } else {
            ([bx - 1, by], [bx, by + 1])
        }
    } else if dy < 1 {
        ([bx - 1, by], [bx, by + 1])
    } else {
        ([bx, by - 1], [bx - 1, by])
    }
}

impl Construction {
    /// `FUN_1411cc430`: `amount` seconds of work on a foundation (state 0, at least 1 hit point;
    /// else the work is done): the first starts it ([`Construction::start`]), each adds its share
    /// of the master's hit points (all of them without a build time) up to the build time, and the
    /// work that reaches it completes the building ([`Construction::complete`]). A building with
    /// annexes is not modelled.
    pub fn work(
        site: &mut Site<'_>,
        mut amount: f32,
        world: &mut Spatial,
        pathing: &mut Pathing,
        now: u32,
    ) -> Result<Worked, Unsupported> {
        if site.life.state != 0 || site.life.hp < 1.0 {
            return Ok(Worked::Done);
        }
        let c = &mut *site.construction;
        if c.desolid {
            Self::start(site.id, c, world, pathing)?;
            // FUN_1411e5670's last line: the last move time is the world's time plus 1
            if let Some(m) = site.mover.as_deref_mut() {
                m.last_move = now.wrapping_add(1);
            }
        }
        let build_time = f32::from(c.build_time);
        let hit_points = f32::from(c.hit_points);
        if build_time < c.progress + amount {
            amount = build_time - c.progress;
        }
        let hp = if 1.0 <= build_time {
            (amount * hit_points) / build_time + site.life.hp
        } else {
            hit_points
        };
        site.life.hp = if hit_points < hp { hit_points } else { hp };
        let before = c.progress;
        c.progress = amount + before;
        if amount + before < build_time {
            return Ok(Worked::Going);
        }
        Self::complete(site, world).map(Worked::Completed)
    }

    /// The construction completes: `FUN_1411cc430`'s `set_state(2)` (`FUN_1411dbb70`), then
    /// `FUN_14152bcb0`, whose part for players with attribute 204 is not modelled. The building
    /// (`FUN_1411c6bd0` first: its player's object index `+0x18` and its count `+0x6c` are not
    /// held; a computer player's, a farm's, an overridden sprite's and a master with a task of
    /// action type 0x99 are not modelled) holds its build time as its build points; its master's
    /// storage of modes 4, 8 and 64 goes to its owner (slot 0xf8, returned); it is built, and its
    /// sight swaps from its footprint to its line of sight (slots 0x1d8, 0x1d0); it takes its
    /// standing sprite (not held); on snow it gets its snow flag; its foundation terrain covers
    /// its clearance ([`crate::terrain::Terrain::cover`]; the tile it stands on, `+0x2e8`, keeps
    /// no terrain to restore in the slice); its owner's attributes 98 and 165 grow by 1; then the
    /// base `set_state` (`FUN_14121cc40`, `FUN_14119d710`) puts it in state 2. The local player's
    /// sound (slot 0x4c0), the event 0x69 (`FUN_140c898a0`) and the owner's AI (slot 0x388) are
    /// not held. Last, the master's technology is researched (`FUN_1413cdcd0` with `param_3` 0
    /// and no building; [`Built::tech`], the caller's). Walls and gates (`adjacent_mode`), a stack
    /// unit, a building that disappears when built, own stores (mode 16), the castle (82), the
    /// monastery (104), the wonder (276) and masters with object flag 0x40 are not modelled.
    pub fn complete(site: &mut Site<'_>, world: &mut Spatial) -> Result<Built, Unsupported> {
        let id = site.id;
        let done = &site.completion;
        let master = done.master.ok_or(Unsupported(
            "a completion of a master the slice does not have",
        ))?;
        let building = master
            .building
            .ok_or(Unsupported("a completion of a master that is no building"))?;
        if done.annexed {
            return Err(Unsupported("a building with annexes completes"));
        }
        if done.attribute_204 {
            return Err(Unsupported(
                "a completion with a player's attribute 204 (FUN_14152bcb0)",
            ));
        }
        if master.class == FARM
            || site.construction.sprite_override
            || master.tasks.iter().any(|t| t.action_type == 0x99)
        {
            return Err(Unsupported(
                "a completion of a farm, an overridden sprite or a task of action type 0x99",
            ));
        }
        if building.adjacent_mode != 0 || building.stack_unit >= 0 || building.disappears_when_built
        {
            return Err(Unsupported(
                "a completion of walls, a stack unit or a vanishing building",
            ));
        }
        if matches!(master.id, 82 | 104 | 276) || master.object_flags & 0x40 != 0 {
            return Err(Unsupported(
                "a completion of the castle, the monastery, the wonder or object flag 0x40",
            ));
        }
        let c = &mut *site.construction;
        c.progress = f32::from(c.build_time);
        let mut attributes = Vec::new();
        for &(kind, amount, flag) in &master.storage {
            if kind < 0 || amount <= 0.0 {
                continue;
            }
            if matches!(flag, 4 | 8 | 64) {
                attributes.push((kind, amount));
            }
            if flag & 0x10 != 0 {
                return Err(Unsupported("a completion with an own store (mode 16)"));
            }
        }
        let placed = *world
            .get(id)
            .ok_or(Unsupported("a completed building without a place"))?;
        if let Some(v) = world.sight.as_mut() {
            v.construction_completed(id, placed.position, placed.radius);
        }
        let terrain = world
            .terrain
            .as_mut()
            .ok_or(Unsupported("a completion without the map's terrain"))?;
        let [x, y, _] = placed.position;
        if !c.snow && 0 <= x as i32 && 0 <= y as i32 && terrain.snow_under(x as i32, y as i32)? {
            c.snow = true;
        }
        if 0 <= building.foundation_terrain {
            terrain.cover(&crate::terrain::Foundation {
                position: placed.position,
                clearance: master.clearance,
                terrain: building.foundation_terrain as u8,
                snow: c.snow,
                master: master.id,
                class: master.class,
                terrain_bonus: master.terrain_bonus,
                mode_680: done.mode_680,
            })?;
        }
        attributes.push((98, 1.0));
        attributes.push((165, 1.0));
        site.life.state = 2;
        world.sight_state(id, 2);
        Ok(Built {
            attributes,
            tech: (0 <= building.tech).then_some(building.tech),
        })
    }

    /// The construction starts (`FUN_1411cd0d0`, then `FUN_1411e5670`): other players' foundations
    /// it overlaps would be removed (not modelled); it is no longer desolid, holds a sliver of
    /// progress, and gets its obstruction record (`FUN_141056460`: the tiles it marks, which the
    /// planners see, and its shape in the mesh); its footprint comes into its player's sight and
    /// its tiles link to its anchor again (`FUN_140f73900`); its last move time becomes the
    /// world's time plus 1 ([`Construction::work`]). Its damage sprites, `+0xa00` and `+0xca4` are
    /// not held by the slice.
    fn start(
        id: i32,
        c: &mut Self,
        world: &mut Spatial,
        pathing: &mut Pathing,
    ) -> Result<(), Unsupported> {
        let placed = *world
            .get(id)
            .ok_or(Unsupported("a foundation that is gone"))?;
        if world.foundations_overlapping(id, &placed) {
            return Err(Unsupported("a construction start over another foundation"));
        }
        c.desolid = false;
        c.progress = f32::from_bits(STARTED);
        if let Some(v) = world.sight.as_mut() {
            v.construction_started(id, placed.position, placed.radius);
        }
        world.add_record(id, c.record);
        pathing.tiles_changed(world.take_changes());
        if let Some(p) = world.get(id) {
            pathing.moved(id, p, false);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A villager's Build task: range 0.1, work value 1, work rate 1.
    const TASK: BuildTask = BuildTask {
        range: 0.1,
        work_value: 1.0,
        work_rate: 1.0,
    };

    /// MarvinBlue/426048731's house 10028 as BUILD placed it: a foundation of master 70.
    fn house() -> crate::spatial::Placed {
        crate::spatial::Placed {
            position: [30.0, 88.0, 0.0],
            radius: [1.0, 1.0],
            class: 3,
            player: 2,
            master: 70,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        }
    }

    /// MarvinBlue/426048731's house 10026 in update 1115 (player 1): the work that reaches 25 s
    /// completes it: its build points hold the build time, its storage (population headroom, 4,
    /// of mode 4) and attributes 98 and 165 go to its owner, it stands in state 2 and its
    /// foundation (27) covers its snow with terrain 6 under layer 32.
    #[test]
    fn the_work_that_reaches_the_build_time_completes_the_building() {
        use crate::spatial::Tiles;
        use crate::terrain::{Terrain, TerrainKind};
        let mut world = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        world.insert(
            10026,
            crate::spatial::Placed {
                position: [2.0, 2.0, 0.0],
                ..house()
            },
        );
        let mut kinds = vec![
            TerrainKind {
                shown: true,
                flags: 0x20
            };
            40
        ];
        kinds[32].flags = 0xa0;
        world.terrain = Some(Terrain::new(
            4,
            4,
            vec![(32, 255, -1); 16],
            kinds,
            vec![vec![0.0; 40]; 5],
        ));
        let master = Master {
            id: 70,
            kind: 80,
            class: 3,
            clearance: [1.0, 1.0],
            terrain_bonus: 4,
            storage: vec![(4, 5.0, 4), (-1, 0.0, 0), (-1, 0.0, 0)],
            building: Some(crate::masters::BuildingMaster {
                adjacent_mode: 0,
                disappears_when_built: false,
                stack_unit: -1,
                foundation_terrain: 27,
                tech: -1,
                garrison_type: 0,
            }),
            ..Master::default()
        };
        let mut life = Lifecycle {
            state: 0,
            hp: 548.0,
            master_id: 70,
            ..Lifecycle::default()
        };
        let mut c = Construction {
            progress: 24.99,
            desolid: false,
            builders: 2,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
        };
        let mut site = Site {
            id: 10026,
            life: &mut life,
            construction: &mut c,
            mover: None,
            completion: Completion {
                master: Some(&master),
                annexed: false,
                attribute_204: false,
                mode_680: 0,
            },
        };
        let worked = Construction::work(&mut site, 0.02, &mut world, &mut Pathing::default(), 0);
        assert_eq!(
            worked,
            Ok(Worked::Completed(Built {
                attributes: vec![(4, 5.0), (98, 1.0), (165, 1.0)],
                tech: None
            }))
        );
        assert_eq!((life.state, c.progress, c.snow), (2, 25.0, true));
        let t = world.terrain.as_ref().unwrap();
        assert_eq!((t.kinds[5], t.layers[5], t.kinds[0]), (6, 32, 32));
    }

    /// MarvinBlue/426048731's house 10028 in update 731: 9177's first work starts it (its record
    /// marks its tiles) and adds 0.021 s of work: 550 hit points over 25 s, so 1.462 hit points.
    #[test]
    fn the_first_work_starts_the_foundation_and_adds_its_share_of_hit_points() {
        use crate::spatial::Tiles;
        let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
        world.insert(10028, house());
        let mut life = Lifecycle {
            state: 0,
            hp: 1.0,
            master_id: 70,
            ..Lifecycle::default()
        };
        let mut c = Construction {
            progress: 0.0,
            desolid: true,
            builders: 1,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
        };
        let mut pathing = Pathing::default();
        let mut site = Site {
            id: 10028,
            life: &mut life,
            construction: &mut c,
            mover: None,
            completion: Completion {
                master: None,
                annexed: false,
                attribute_204: false,
                mode_680: 0,
            },
        };
        let rate = 1.0f32 * 1.0 * 0.021;
        assert_eq!(
            Construction::work(&mut site, rate, &mut world, &mut pathing, 0),
            Ok(Worked::Going)
        );
        assert_eq!(life.hp, 1.462);
        assert!(!c.desolid);
        assert_eq!(c.progress, f32::from_bits(STARTED) + rate);
        assert!(world.get(10028).unwrap().obstruction.is_some());
        assert!(world.tiles().word(29, 87).unwrap() & crate::spatial::BLOCKED != 0);
    }
    use crate::movement::Me;
    use crate::spatial::Spatial;

    /// MarvinBlue/426048731 after tick 679: builder 9175 stands on the house foundation 10028's
    /// footprint, works on it (state 6) and walks off it in update 681 (state 223) to the free tile
    /// the game picks, (29.5, 89.5).
    #[test]
    fn a_builder_on_its_footprint_faces_it_then_walks_to_a_free_tile_beside_it() {
        use crate::movement::{MoveStats, Mover};
        use crate::spatial::{Placed, Tiles};
        let mut world = Spatial::with_tiles([40, 120], Tiles::new(40, 120, vec![4; 40 * 120]));
        let unit = |x, y, radius, master, kind| Placed {
            position: [x, y, 0.0],
            radius: [radius, radius],
            class: 4,
            player: 2,
            master,
            obstruction_type: kind,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        };
        world.insert(9175, unit(30.0, 89.0, 0.2, 118, 5));
        world.insert(10028, unit(30.0, 88.0, 1.0, 70, 2));
        let mut mover = Mover {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, -1.0],
            velocity: None,
            paths: vec![],
            factor: 1.0,
            stats: MoveStats {
                speed: 0.8,
                turn_rate: f32::MAX,
                turn_radius: 0.0,
                turn_radius_speed: f32::MAX,
                radius: [0.2, 0.2, 0.5],
                restriction: 7,
                restriction_mask: 6,
                turn_time: 0.0,
                move_algorithm: 0,
                min_collision: 1.0,
                fast_turn: false,
                walk_speed: 1.0,
                stand_speed: 1.0,
                run_speed: 1.0,
                facets: Some(crate::facing::SpriteFacets {
                    count: 16,
                    flag_1: true,
                }),
                weapon_range: 0.0,
            },
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: 0,
            substitute_count: 0,
            ignore_units: Vec::new(),
            last_turn: 0.0,
            angle: 0.0,
            move_to_point: None,
            speed_stopped: false,
            pauses: Vec::new(),
        };
        let mut build = BuildAction {
            target: 10028,
            state: 6,
            move_to: None,
            goal: None,
            task: TASK,
            added: false,
        };
        let mut life = Lifecycle {
            state: 0,
            ..Lifecycle::default()
        };
        let mut construction = Construction {
            progress: 0.0,
            desolid: true,
            builders: 0,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
        };
        let site = Site {
            id: 10028,
            life: &mut life,
            construction: &mut construction,
            mover: None,
            completion: Completion {
                master: None,
                annexed: false,
                attribute_204: false,
                mode_680: 0,
            },
        };
        let _ = build
            .update(
                Walk {
                    mover: Some(&mut mover),
                    me: Me {
                        id: 9175,
                        position: [30.0, 89.0, 0.0],
                        action_state: 0,
                    },
                    dt: 0.02,
                    world: &mut world,
                    plan: None,
                },
                Some(site),
                Builder {
                    class: VILLAGER,
                    held: 0.0,
                    ai: None,
                },
            )
            .unwrap();
        assert_eq!(
            (build.state, build.move_to, build.goal),
            (223, Some(MoveTo::Start), Some([29.5, 89.5, 0.0]))
        );
    }

    #[test]
    fn a_builder_at_the_foundation_works_in_its_first_update_the_others_walk_to_it() {
        fn walk(id: i32, world: &mut Spatial) -> Walk<'_> {
            Walk {
                mover: None,
                me: Me {
                    id,
                    position: [0.0; 3],
                    action_state: 0,
                },
                dt: 0.02,
                world,
                plan: None,
            }
        }
        let mut near = BuildAction::start(10028, 0.0, TASK);
        let mut far = BuildAction::start(10028, 0.8, TASK);
        assert_eq!((near.state, near.move_to), (4, Some(MoveTo::Arrived)));
        assert_eq!((far.state, far.move_to), (4, Some(MoveTo::Start)));
        let mut world = Spatial::default();
        let builder = || Builder {
            class: VILLAGER,
            held: 0.0,
            ai: None,
        };
        world.insert(10028, house());
        let _ = near.update(walk(1, &mut world), None, builder()).unwrap();
        assert_eq!((near.state, near.move_to), (6, None));
        // the far one checks its foundation, which must be in the world
        let mut empty = Spatial::default();
        assert_eq!(
            far.update(walk(2, &mut empty), None, builder()),
            Err(Unsupported("a Build whose foundation is gone"))
        );
        assert_eq!(far.move_to, Some(MoveTo::Start));
    }
}
