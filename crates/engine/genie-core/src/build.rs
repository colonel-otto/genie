//! A builder's Build action (`TRIBE_Action_Build`, type 101, vtable `0x142ed6628`) up to the
//! builder's walk: the action starts in state 4 with a MoveTo sub-action (`RGE_Action_Move_To`,
//! type 1) toward the foundation; a builder already at the foundation works in its first update
//! (state 6). A walking builder's MoveTo runs the motion step ([`crate::movement`]).
//!
//! A builder at a foundation faces it and joins its work (state 7): at a foundation no one started
//! it first checks the footprint, and a builder standing on it walks to a free tile beside it
//! (state 223) and comes back (state 4). The work adds to the foundation's construction each
//! update ([`Construction::work`]); the first work starts it: the foundation gets its obstruction
//! record. Its completion is not modelled.

use crate::lifecycle::Lifecycle;
use crate::move_to::{Goal, start_walk, walk_on};
pub use crate::move_to::{MoveTo, Walk};
use crate::movement::footprint_gap;
use crate::pathing::Pathing;
use crate::spatial::{Obstruction, Spatial};
use crate::unit_ai::Unsupported;

/// The Build action's type, which word 9 adds.
pub const BUILD: i16 = 101;

/// The gap below which a MoveTo has arrived (`DAT_142eb99cc`).
const ARRIVED: f32 = 0.01;
/// The villager's class (4, `-0xea8` encoded): Build's walk off the footprint returns at once.
const VILLAGER: i16 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildAction {
    /// The foundation's id.
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    pub move_to: Option<MoveTo>,
    /// The point the MoveTo walks to (its `+0x38`..`+0x40`), when not to the foundation.
    pub goal: Option<[f32; 3]>,
    pub task: BuildTask,
    /// `+0x68`, `added_to_num_builders`: the builder counts among the foundation's builders.
    pub added: bool,
}

/// What a builder's Build task gives its work: the task's work range (`+0x30`; its MoveTo to the
/// foundation stops this near) and work value (`+0x28`, `work_val1`), and the builder's work rate:
/// its master's `work_rate` (`+0x798`) plus its own `mWorkRatePowerup` (`+0xa30`, slot 0x6e8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildTask {
    pub range: f32,
    pub work_value: f32,
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
    pub build_time: i16,
    /// The record its start adds: its master's obstruction kind and class.
    pub record: Obstruction,
}

/// What a builder's work reaches of its foundation.
pub struct Site<'a> {
    pub id: i32,
    pub life: &'a mut Lifecycle,
    pub construction: &'a mut Construction,
    /// The foundation's movement state (its last move time), if the slice holds it.
    pub mover: Option<&'a mut crate::movement::Mover>,
}

/// A second builder works at a third of the rate of the first (`DAT_142ed66b0`).
const EXTRA_BUILDER: f32 = 0.33;
/// The progress a started foundation holds before its first work (`0x2edbe6ff`, about 1e-10).
const STARTED: u32 = 0x2edb_e6ff;
/// The Wonder's master (`0x114`), whose construction rate the player's attribute raises at most to
/// `DAT_142ed66b4`.
const WONDER: i16 = 0x114;
const WONDER_RATE: f32 = 1.2;
/// The wall class (27, `0x1b`): two walls beside a builder on a footprint's diagonal stop it.
const WALL: i16 = 27;

/// The Build action's states the slice models.
const MOVE: u8 = 4;
const WORK: u8 = 6;
const PLACE: u8 = 7;
/// 223 (`0xdf`): off the footprint, to a free tile beside it.
const CLEAR: u8 = 223;

impl BuildAction {
    /// The action's start (slot 5, `0x140aadd80`): state 4 for a target, whose set_state
    /// (`0x140aae200`) creates the MoveTo; the MoveTo starts arrived if the footprints touch.
    pub fn start(target: i32, gap: f32, task: BuildTask) -> Self {
        BuildAction {
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

    /// One update (slot 6, `0x140ab0370`), which first updates the MoveTo (`0x140aa2a70`); a
    /// moving MoveTo calls the motion step (object slot 193). `site` is the foundation, when it
    /// is the builder's player's.
    pub fn update(&mut self, walk: Walk<'_>, site: Option<Site<'_>>) -> Result<(), Unsupported> {
        match (self.state, self.move_to) {
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
            (4, Some(MoveTo::Moving)) => {
                self.move_to = Some(walk_on(walk)?);
                Ok(())
            }
            (4, None) => Err(Unsupported("a Build action in state 4 without its MoveTo")),
            (WORK, _) => {
                // case 5: FUN_141162db0 turns the builder to face its foundation
                let mover = walk
                    .mover
                    .as_deref()
                    .ok_or(Unsupported("a builder without movement state"))?;
                if !mover.stats.turns_at_once {
                    return Err(Unsupported(
                        "a builder that turns over time to face its work",
                    ));
                }
                self.place(&walk, site)
            }
            (PLACE, _) => self.work(walk, site),
            (CLEAR, Some(MoveTo::Start)) => {
                self.move_to = Some(MoveTo::Moving);
                let goal = self.goal.expect("set_state(223) sets the goal");
                self.move_to = Some(start_walk(walk, Goal::Point(goal), 0.0, false)?);
                Ok(())
            }
            (CLEAR, Some(MoveTo::Moving)) => {
                self.move_to = Some(walk_on(walk)?);
                Ok(())
            }
            // case 0xde: the MoveTo ended (its update in state 1 returns 1): a villager goes
            // back to the foundation (set_state 4)
            (CLEAR, Some(MoveTo::Arrived)) => self.back(&walk),
            (CLEAR, _) => Err(Unsupported(
                "a builder off its footprint without its MoveTo",
            )),
            _ => Err(Unsupported("a Build state not modelled")),
        }
    }

    /// set_state(7) (`0x140aae200` case 7) on a foundation no builder started: clear the
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
    /// tiles beside it on the footprint's diagonal (set_state 2, not modelled), it counts among
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
    fn work(&mut self, walk: Walk<'_>, site: Option<Site<'_>>) -> Result<(), Unsupported> {
        let site = site.ok_or(Unsupported("a foundation of another player"))?;
        let plan = walk
            .plan
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
        let done = Construction::work(site, rate, walk.world, plan.pathing, plan.now)?;
        if done {
            return Err(Unsupported(
                "a finished construction (Build set_state 0xe0)",
            ));
        }
        Ok(())
    }

    /// set_state(4) after the walk off the footprint (`0x140aae200` case 4): the goal is the
    /// foundation again and a new MoveTo to it (`FUN_140aa0df0`) starts arrived if the footprints
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
        *self = BuildAction::start(self.target, gap, self.task);
        Ok(())
    }

    /// set_state(223): a MoveTo to the free tile beside the footprint (`FUN_1411566b0`); it starts
    /// in state 3, as a MoveTo to a point is far from it (`FUN_140aa2100`).
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
fn beside_diagonal(bx: i32, by: i32, dx: i32, dy: i32) -> ([i32; 2], [i32; 2]) {
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
    /// of the master's hit points (all of them without a build time) up to the build time, and
    /// true when the work reaches it (its completion, set_state 2, is not modelled). A building
    /// with annexes is not modelled.
    pub fn work(
        site: Site<'_>,
        mut amount: f32,
        world: &mut Spatial,
        pathing: &mut Pathing,
        now: u32,
    ) -> Result<bool, Unsupported> {
        if site.life.state != 0 || site.life.hp < 1.0 {
            return Ok(true);
        }
        let c = &mut *site.construction;
        if c.desolid {
            Construction::start(site.id, c, world, pathing)?;
            // FUN_1411e5670's last line: the last move time is the world's time plus 1
            if let Some(m) = site.mover {
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
            return Ok(false);
        }
        Err(Unsupported(
            "a building's construction completes (set_state 2)",
        ))
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
        c: &mut Construction,
        world: &mut Spatial,
        pathing: &mut Pathing,
    ) -> Result<(), Unsupported> {
        let placed = world
            .get(id)
            .ok_or(Unsupported("a foundation that is gone"))?
            .clone();
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

    /// MarvinBlue/426048731's house 10028 in update 731: 9177's first work starts it (its record
    /// marks its tiles) and adds 0.021 s of work: 550 hit points over 25 s, so 1.462 hit points.
    #[test]
    fn the_first_work_starts_the_foundation_and_adds_its_share_of_hit_points() {
        use crate::spatial::{Placed, Tiles};
        let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
        world.insert(
            10028,
            Placed {
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
            },
        );
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
        };
        let mut pathing = Pathing::default();
        let site = Site {
            id: 10028,
            life: &mut life,
            construction: &mut c,
            mover: None,
        };
        let rate = 1.0f32 * 1.0 * 0.021;
        assert_eq!(
            Construction::work(site, rate, &mut world, &mut pathing, 0),
            Ok(false)
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
                turns_at_once: true,
                move_algorithm: 0,
                min_collision: 1.0,
            },
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: 0,
            substitute_count: 0,
        };
        let mut build = BuildAction {
            target: 10028,
            state: 6,
            move_to: None,
            goal: None,
            task: TASK,
            added: false,
        };
        let mut life = Lifecycle::default();
        let mut construction = Construction {
            progress: 0.0,
            desolid: true,
            builders: 0,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
        };
        let site = Site {
            id: 10028,
            life: &mut life,
            construction: &mut construction,
            mover: None,
        };
        build
            .update(
                Walk {
                    mover: Some(&mut mover),
                    me: Me {
                        id: 9175,
                        position: [30.0, 89.0, 0.0],
                    },
                    dt: 0.02,
                    world: &mut world,
                    plan: None,
                },
                Some(site),
            )
            .unwrap();
        assert_eq!(
            (build.state, build.move_to, build.goal),
            (223, Some(MoveTo::Start), Some([29.5, 89.5, 0.0]))
        );
    }

    #[test]
    fn a_builder_at_the_foundation_works_in_its_first_update_the_others_walk_to_it() {
        let mut near = BuildAction::start(10028, 0.0, TASK);
        let mut far = BuildAction::start(10028, 0.8, TASK);
        assert_eq!((near.state, near.move_to), (4, Some(MoveTo::Arrived)));
        assert_eq!((far.state, far.move_to), (4, Some(MoveTo::Start)));
        let mut world = Spatial::default();
        fn walk(id: i32, world: &mut Spatial) -> Walk<'_> {
            Walk {
                mover: None,
                me: Me {
                    id,
                    position: [0.0; 3],
                },
                dt: 0.02,
                world,
                plan: None,
            }
        }
        near.update(walk(1, &mut world), None).unwrap();
        assert_eq!((near.state, near.move_to), (6, None));
        // the far one asks for a path to its foundation, which must be on the map
        assert_eq!(
            far.update(walk(2, &mut world), None),
            Err(Unsupported("a MoveTo whose target is gone"))
        );
        assert_eq!(far.move_to, Some(MoveTo::Moving));
    }
}
