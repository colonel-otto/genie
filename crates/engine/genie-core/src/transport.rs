//! The Transport action (`RGE_Action_Transport`, type 12, vtable `0x142ed61e8`): a transport's
//! sail to a point and its unload there. The unit's unload command (slot 0x3a8, `FUN_14108d050`,
//! which the unit AI's unload order 0x2d1 runs) makes one at the unit's place and gives it the
//! order's point as its work ([`TransportAction::work`]): near enough it unloads at once (state
//! 6), else it sails toward the point (state 4, a quiet `MoveTo` sub-action to the last point of
//! the planner's path that a unit can cross, [`Shore::shore_point`]) and unloads where that ends.
//! The unload (`FUN_140aa3ba0`, the caller's) puts the units inside ashore where a drop location
//! is free ([`find_drop_location`]); when none goes, the action sails to a random point about
//! the old one ([`retry_point`]) and tries again.
//!
//! 158041: the constructor `FUN_140aa3ab0`, `first_in_stack` `0x140aa3e10` (slot 5), update
//! `0x140aa4640` (slot 6), stop `0x140aa4620` (slot 7), `move_to` `0x140aa3e40` (slot 8), work
//! `0x140aa4830` (slot 9), `set_state` `0x140aa42a0` (slot 16), the shore search `FUN_140aa3f40`
//! (HD's `next_to_drop_off_terrain`).

use crate::GenieError;
use crate::checkpoint::cvttss2si;
use crate::move_to::{Goal, MoveTo, Walk, start_walk_asked, walk_or_give_up};
use crate::movement::PathAsk;
use crate::rng::Well512;

/// The Transport action's type, which word 9 adds.
pub const TRANSPORT: i16 = 12;
/// The transport's action value (`0x266`), which its notifies carry and the unit AI holds while
/// it transports or unloads.
pub const TRANSPORT_ACTION: i32 = 0x266;

/// The action's point while it has none (`DAT_14330ec20`).
pub const NONE: f32 = crate::image::transport::NONE;
/// The reach within which a tile of the point's zone lets the unit unload (`DAT_142e8c150`).
const NEAR: f32 = crate::image::transport::NEAR;
/// The shore search's margin about the unit's footprint (`DAT_142e8c150`).
const SHORE_MARGIN: f32 = crate::image::transport::SHORE_MARGIN;
/// The squared gap from the unit's footprint within which the work unloads at once
/// (`DAT_142eb0f48`).
const UNLOAD_REACH_SQUARED: f32 = crate::image::transport::UNLOAD_REACH_SQUARED;
/// The `MoveTo`'s range to the point (`DAT_142e8c148`).
const MOVE_RANGE: f32 = crate::image::transport::MOVE_RANGE;
/// The new point's spread about the old one (`DAT_14330ebb8`): a draw over [`RETRY_STEP`] less
/// this.
const RETRY_SPREAD: f32 = crate::image::transport::RETRY_SPREAD;
/// The divisor that cuts a 15-bit draw to 0..10 tiles (`FUN_140aa3ba0`'s `0xccc`).
const RETRY_STEP: u32 = 0xccc;
/// The new point's draws (`FUN_140aa3ba0`).
const RETRIES: i32 = 100;
/// The terrains a new point must leave: 22, 23 and 1, the waters (`FUN_140aa3ba0`).
const DEEP_WATER: u8 = 0x16;
const WATER: u8 = 1;
/// A drop location's draw scale (`DAT_142eb69cc`) and the gap past the footprints
/// (`DAT_142eb99cc`).
const DRAW_SCALE: f32 = crate::image::transport::DRAW_SCALE;
const DROP_GAP: f32 = crate::image::transport::DROP_GAP;
/// The draws `find_drop_location` makes (`FUN_141192d10`: 50).
const DROP_TRIES: u16 = 50;

/// The terrain table's water bits (`FUN_1413fc2b0`) and land bits (`FUN_1413fc270`).
const WATER_FLAGS: u8 = 7;
const LAND_FLAGS: u8 = 0x18;

/// The Transport's states.
pub const DONE: u8 = 1;
/// Idle: the update does nothing.
pub const IDLE: u8 = 2;
/// Sailing to unload.
pub const SAIL_TO_UNLOAD: u8 = 4;
/// Unloading.
pub const UNLOAD: u8 = 6;
/// Sailing.
pub const SAIL: u8 = 11;
/// Failed: the update returns 3.
pub const FAILED: u8 = 13;

/// A Transport's unit of a master the slice does not have.
pub const NO_MASTER: GenieError =
    GenieError::unsupported("a Transport's unit of a master the slice does not have");

/// A drop location's first draw of side 4 (a draw of `0x7fff`), which keeps the place the
/// stack holds: not modelled.
pub const STACK_PLACE: GenieError =
    GenieError::unsupported("a drop location's first draw of side 4 (the place the stack holds)");

/// A unit's Transport action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransportAction {
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// `+0x34`: −1 from the constructor (the base's `+0x30` pair), 1 after a work order (slot 9),
    /// 0 after a move (slot 8).
    pub mode: i32,
    /// `+0x38`..`+0x40`: where it sails or unloads; x [`NONE`] for nowhere.
    pub point: [f32; 3],
    /// Its quiet `MoveTo` sub-action's state, which walks to the point within [`MOVE_RANGE`].
    pub move_to: Option<MoveTo>,
    /// `+0x68`: the unload found no drop location and the action sails to a new point.
    pub retarget: bool,
}

/// What the Transport reads of its unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ship {
    /// The unit's place.
    pub position: [f32; 3],
    /// Its master's radii (x, y).
    pub radius: [f32; 2],
    /// An object is inside it (`+0x2f8` holds one).
    pub loaded: bool,
    /// The map's size in tiles (the owner's world's map, `+0x18`, `+0x1c`).
    pub map: [i32; 2],
    /// It has speed (slot 0x208, `UCOMISS` with 0: a NaN has), which its `MoveTo` needs.
    pub moves: bool,
}

/// What the Transport reads of the map about its unit (the caller's).
pub trait Shore {
    /// `FUN_1411958c0`: the unit's zone (its restriction's zone map, `FUN_1411931c0`) at tile
    /// (`x`, `y`): 0 for a tile below 0, or without a zone map.
    ///
    /// # Errors
    ///
    /// A map the caller cannot read.
    fn zone(&mut self, x: i32, y: i32) -> Result<u16, GenieError>;
    /// `FUN_1411a7570`: a tile of zone `zone` lies within `reach` of the unit's tile (its place
    /// cut by `CVTTSS2SI`, [`crate::zones::ZoneMap::near`]); no without a zone map.
    ///
    /// # Errors
    ///
    /// A map the caller cannot read.
    fn near(&mut self, zone: u8, reach: f32) -> Result<bool, GenieError>;
    /// The terrain table's flags of tile (`x`, `y`) ([`crate::terrain::Terrain::flags_at`]).
    fn flags(&self, x: i16, y: i16) -> u8;
    /// `FUN_140f7c390` with the planner of the unit's restriction (`FUN_140f7cb30`, none without
    /// a master): the last point of the planner's path from `from` to `to` whose tile has an edge
    /// ([`crate::tile_path::Planner::last_open_point`]); None without a planner or such a point.
    fn shore_point(&mut self, from: [f32; 2], to: [f32; 2]) -> Option<[f32; 2]>;
}

/// What a state change leaves to its caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// A `MoveTo` sub-action went (`FUN_140a98620`): its teardown stops the unit (slot 0x5d0).
    pub stopped: bool,
    /// State 2: the unit shows its master's standing sprite (slot 0x68 with `+0x470`).
    pub stands: bool,
}

/// What an update leaves to its caller ([`TransportAction::update`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransportStep {
    /// The update's code: 1 done, 3 failed (the action list removes it), else 0.
    pub code: u8,
    /// What its state changes left ([`Changed`]).
    pub changed: Changed,
    /// State 13: the unit's notify 0x1f9 with [`TRANSPORT_ACTION`] (slot 0x328, to its unit AI).
    pub failed: bool,
    /// The owner hears 0x207 (its slot 0x2e8 with [`TRANSPORT_ACTION`]): no unload there.
    pub cannot_unload: bool,
    /// The unload (`FUN_140aa3ba0`) is the caller's, then [`TransportAction::unloaded`].
    pub unload: bool,
}

impl TransportAction {
    /// The unload command's constructor (`FUN_140aa3ab0` with no task, `+0x48`): its point the
    /// unit's place `at`, state 0 and `+0x34` −1 (the base's, `FUN_140a73e90`), `+0x68` clear.
    #[must_use]
    pub const fn new(at: [f32; 3]) -> Self {
        Self {
            state: 0,
            mode: -1,
            point: at,
            move_to: None,
            retarget: false,
        }
    }

    /// Slot 5 (`FUN_140aa3e10`), the start: `first` (the action list's add passes 1) with a
    /// point not below 0 on either axis (`COMISS`/`JA`, `JBE`: a NaN is not) sails there
    /// (`set_state(11)`); else it idles (`set_state(2)`).
    pub fn start(&mut self, first: bool, ship: &Ship, shore: &mut impl Shore) -> Changed {
        let [x, y, _] = self.point;
        let sails = first
            && crate::fmath::at_most_or_unordered(0.0, x)
            && crate::fmath::at_most_or_unordered(0.0, y);
        self.set_state(if sails { SAIL } else { IDLE }, ship, shore)
    }

    /// `set_state` (slot 16, `FUN_140aa42a0`): its sub-actions go first (`FUN_140a98620`: a
    /// `MoveTo`'s teardown stops the unit, [`Changed::stopped`]), then the state. State 2 shows
    /// the master's standing sprite ([`Changed::stands`]). States 4 and 11 sail: the point
    /// becomes the shore point on the way from the unit's place ([`Shore::shore_point`]) when
    /// there is one; then a quiet `MoveTo` to the point within [`MOVE_RANGE`] (`FUN_140aa0d30`
    /// with no task: the master's `+0x748` sprite; `FUN_140a7a170` with 1) joins the list
    /// (`FUN_140a977e0`, for a unit in a state below 3) and starts (`0x140aa2100`: a point is
    /// never within its start's gap, so state 3, or 13 for a unit without speed, which a quiet
    /// `MoveTo` shows no sprite for).
    pub fn set_state(&mut self, state: u8, ship: &Ship, shore: &mut impl Shore) -> Changed {
        let mut changed = Changed {
            stopped: self.move_to.take().is_some(),
            stands: false,
        };
        self.state = state;
        match state {
            IDLE => changed.stands = true,
            SAIL_TO_UNLOAD | SAIL => {
                let [ux, uy, _] = ship.position;
                let to = [self.point[0], self.point[1]];
                if let Some([sx, sy]) = shore.shore_point([ux, uy], to) {
                    self.point[0] = sx;
                    self.point[1] = sy;
                }
                self.move_to = Some(if ship.moves {
                    MoveTo::Start
                } else {
                    MoveTo::Failed
                });
            }
            _ => {}
        }
        changed
    }

    /// Slot 8 (`FUN_140aa3e40`), a move to `point` (x, y and the height; the object it names
    /// is not read): the point it sails to, `+0x34` 0, then `set_state(11)`; it takes the move
    /// (1). Its sprite field (`+0x58`, the master's `+0x748` or standing sprite) is the
    /// interface's.
    pub fn move_to(&mut self, point: [f32; 3], ship: &Ship, shore: &mut impl Shore) -> Changed {
        self.point = point;
        self.mode = 0;
        self.set_state(SAIL, ship, shore)
    }

    /// Slot 7 (`FUN_140aa4620`), the stop: `set_state(2)`.
    pub fn stop(&mut self, ship: &Ship, shore: &mut impl Shore) -> Changed {
        self.set_state(IDLE, ship, shore)
    }

    /// Slot 9 (`FUN_140aa4830`), the work order on `point` (x, y and the height), `by_self` when
    /// the object it names is the unit itself (the unload command's); `+0x34` becomes 1. Near
    /// enough to unload there — a tile of the zone of the point's tile (`CVTTSS2SI`,
    /// [`Shore::zone`]) within [`NEAR`] of the unit ([`Shore::near`], exactly 1), and the point
    /// within [`UNLOAD_REACH_SQUARED`] of the unit's footprint on the ground (each axis' gap past
    /// the master's radius, `MAXSS` with 0: a NaN gap stays; `COMISS`/`JC`: a NaN is not
    /// within) — it takes the point and unloads (`set_state(6)`). Else a point below 0 on either
    /// axis (`COMISS`/`JC`: a NaN too) changes nothing; any other becomes the point, and the
    /// unit sails there to unload (`set_state(4)`) from the unit itself, else just sails (11).
    ///
    /// # Errors
    ///
    /// What the map reads give.
    pub fn work(
        &mut self,
        point: [f32; 3],
        by_self: bool,
        ship: &Ship,
        shore: &mut impl Shore,
    ) -> Result<Changed, GenieError> {
        self.mode = 1;
        let [x, y, _] = point;
        let zone = shore.zone(cvttss2si(x), cvttss2si(y))?;
        let [ux, uy, _] = ship.position;
        // MAXSS 0, gap: the gap when it is not below 0, a NaN gap too
        let dx = crate::fmath::maxss(0.0, (x - ux).abs() - ship.radius[0]);
        let dy = crate::fmath::maxss(0.0, (y - uy).abs() - ship.radius[1]);
        let near = shore.near(zone as u8, NEAR)?;
        let within = !crate::fmath::below_or_unordered(UNLOAD_REACH_SQUARED, dy * dy + dx * dx);
        let state = if near && within {
            UNLOAD
        } else if crate::fmath::below_or_unordered(x, 0.0)
            || crate::fmath::below_or_unordered(y, 0.0)
        {
            return Ok(Changed::default());
        } else if by_self {
            SAIL_TO_UNLOAD
        } else {
            SAIL
        };
        self.point = point;
        Ok(self.set_state(state, ship, shore))
    }

    /// One update (slot 6, `FUN_140aa4640`) after its `MoveTo`'s (`walked`, the code of
    /// [`TransportAction::walk`], which only states 4 and 11 read). State 1 is done (1). States 4
    /// and 11: any end of the `MoveTo` (codes 1 to 5; without one the list answers 1) unloads
    /// (4: `set_state(6)`) or idles (11: `set_state(2)`). State 13 tells the unit
    /// it failed ([`TransportStep::failed`]), idles and returns 3. State 6 unloads:
    /// - with a point, its zone's tile ([`Shore::zone`]) must lie within [`NEAR`] of the unit
    ///   ([`Shore::near`]), else the owner hears that it cannot unload there
    ///   ([`TransportStep::cannot_unload`]); `+0x68` clears either way.
    /// - with units inside, a unit beside the shore ([`TransportAction::next_to_shore`]) unloads
    ///   ([`TransportStep::unload`], the caller's, then [`TransportAction::unloaded`]); one that
    ///   is not, over water at the point (`FUN_1413fbf30`, `FUN_1413fc2b0`: the terrain table's
    ///   bits 0 to 2 at the point's tile) waits there, else the owner hears that it cannot unload.
    /// - unless the unload sails on (`+0x68`), the action idles (`set_state(2)`).
    ///
    /// # Errors
    ///
    /// What the map reads and the `MoveTo` give.
    pub fn update(
        &mut self,
        walked: u8,
        ship: &Ship,
        shore: &mut impl Shore,
    ) -> Result<TransportStep, GenieError> {
        let mut step = TransportStep::default();
        match self.state {
            DONE => step.code = 1,
            SAIL_TO_UNLOAD | SAIL => {
                if (1..=5).contains(&walked) {
                    let next = if self.state == SAIL { IDLE } else { UNLOAD };
                    step.changed = self.set_state(next, ship, shore);
                }
            }
            FAILED => {
                step.failed = true;
                step.changed = self.set_state(IDLE, ship, shore);
                step.code = 3;
            }
            UNLOAD => {
                let [x, y, _] = self.point;
                let mut reaches = true;
                if x == NONE {
                    self.retarget = false;
                } else {
                    let zone = shore.zone(cvttss2si(x), cvttss2si(y))?;
                    self.retarget = false;
                    reaches = shore.near(zone as u8, NEAR)?;
                }
                if !reaches {
                    step.cannot_unload = true;
                } else if ship.loaded {
                    if self.next_to_shore(ship, shore)? {
                        step.unload = true;
                        return Ok(step);
                    }
                    let over_water = x != NONE
                        && shore.flags(cvttss2si(x) as i16, cvttss2si(y) as i16) & WATER_FLAGS != 0;
                    step.cannot_unload = !over_water;
                }
                step.changed = self.unloaded(ship, shore);
            }
            _ => {}
        }
        Ok(step)
    }

    /// The end of state 6's update after the unload ([`TransportStep::unload`]): unless the
    /// unload sails on (`+0x68`), `set_state(2)`.
    pub fn unloaded(&mut self, ship: &Ship, shore: &mut impl Shore) -> Changed {
        if self.retarget {
            return Changed::default();
        }
        self.set_state(IDLE, ship, shore)
    }

    /// The update of its `MoveTo` sub-action (`FUN_140a992b0`) with `walk` and what the unit's
    /// AI asks of the path (`ask`, `0x140aa2a70` in state 3), as [`crate::artifact::Artifact`]
    /// runs its own: state 3 asks for its path to the point within [`MOVE_RANGE`] (quiet), state
    /// 4 moves, a motion step that gives up fails it (`set_state(13)`: only a human player's
    /// villager waits); its code: 1 once it ended at the point, 3 once it failed, 1 without one
    /// ("There is no list"), else 0.
    ///
    /// # Errors
    ///
    /// A walk the `MoveTo` needs and has not got, and what the motion step does not model.
    pub fn walk(&mut self, walk: Option<Walk<'_>>, ask: PathAsk) -> Result<u8, GenieError> {
        let need = || GenieError::unreachable("a sailing Transport without its walk");
        match self.move_to {
            None | Some(MoveTo::Arrived) => return Ok(1),
            Some(MoveTo::Failed) => return Ok(3),
            Some(MoveTo::Start) => {
                let goal = Goal::Point(self.point);
                let state = start_walk_asked(walk.ok_or_else(need)?, goal, MOVE_RANGE, true, ask)?;
                self.move_to = Some(state.unwrap_or(MoveTo::Failed));
            }
            Some(MoveTo::Moving) => {
                let state = walk_or_give_up(walk.ok_or_else(need)?)?;
                self.move_to = Some(state.unwrap_or(MoveTo::Failed));
            }
            Some(MoveTo::Waiting) => {
                return Err(GenieError::unreachable(
                    "a Transport's MoveTo waiting (only a villager's waits)",
                ));
            }
        }
        Ok(0)
    }

    /// `FUN_140aa3f40` (HD's `next_to_drop_off_terrain`), without a task (`+0x48`; a task whose
    /// `+0x16` is −1 would answer yes): a tile about the unit's footprint grown by
    /// [`SHORE_MARGIN`] (each edge cut by `CVTTSS2SI`, kept on the map) of the point's zone
    /// ([`Shore::zone`]), or without a point (x [`NONE`]) a land tile (the terrain table's bits 3
    /// and 4, `FUN_1413fbee0` and `FUN_1413fc270`); row by row, each from its least x.
    ///
    /// # Errors
    ///
    /// What the map reads give.
    pub fn next_to_shore(&self, ship: &Ship, shore: &mut impl Shore) -> Result<bool, GenieError> {
        let [ux, uy, _] = ship.position;
        let [rx, ry] = ship.radius;
        let [w, h] = ship.map;
        let x0 = cvttss2si((ux - rx) - SHORE_MARGIN).max(0);
        let y0 = cvttss2si((uy - ry) - SHORE_MARGIN).max(0);
        let x1 = cvttss2si((ux + rx) + SHORE_MARGIN).min(w - 1);
        let y1 = cvttss2si((uy + ry) + SHORE_MARGIN).min(h - 1);
        let [x, y, _] = self.point;
        let zone = if x == NONE {
            None
        } else {
            Some(shore.zone(cvttss2si(x), cvttss2si(y))?)
        };
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let found = match zone {
                    None => shore.flags(tx as i16, ty as i16) & LAND_FLAGS != 0,
                    Some(z) => shore.zone(tx, ty)? == z,
                };
                if found {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

/// `FUN_140aa3ba0`'s new point once no unit went ashore: from the old point (x, y), draws of the
/// Action stream ("Unload": x, then y), each `r / 0xccc` tiles plus the old coordinate less
/// [`RETRY_SPREAD`], while the terrain a map read sees at the last draw's tile (`terrain`, kept
/// while the tile is off the map) is water (22, 23 or 1), at most 100; the last draw becomes the
/// point either way. It sails on (true) only when a draw left the water with more than one draw
/// to spare: the 99th and 100th never count.
pub fn retry_point(
    point: [f32; 2],
    map: [i32; 2],
    terrain: impl Fn(i16, i16) -> u8,
    rng: &mut Well512,
) -> ([f32; 2], bool) {
    let [w, h] = map;
    let mut seen = DEEP_WATER;
    let mut at = point;
    let mut left = RETRIES;
    loop {
        let tries = left;
        left = tries - 1;
        if !matches!(seen.wrapping_sub(DEEP_WATER), 0 | 1) && seen != WATER {
            break;
        }
        let x = (rng.rand15() / RETRY_STEP) as f32 + point[0] - RETRY_SPREAD;
        let y = (rng.rand15() / RETRY_STEP) as f32 + point[1] - RETRY_SPREAD;
        at = [x, y];
        let (tx, ty) = (cvttss2si(x), cvttss2si(y));
        if 0 <= tx && 0 <= ty && tx < w && ty < h {
            seen = terrain(tx as i16, ty as i16);
        }
        if left <= 0 {
            left = tries - 2;
            break;
        }
    }
    (at, 0 < left)
}

/// Where [`find_drop_location`] puts a unit: about a centre within radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Around {
    /// The centre (x, y).
    pub center: [f32; 2],
    /// The radii (x, y).
    pub radius: [f32; 2],
}

/// `FUN_141192d10` (`find_drop_location`) for a unit of master radii `unit_radius` about
/// `around`: up to 50 draws of the Object stream ("`find_drop_location`"). Each picks a side
/// (`r * 4 / 0x7fff`: east, west, north or south past the radii, the unit's own and
/// [`DROP_GAP`]), then a place along it (`r * 2 radius / 32767 - radius`, [`DRAW_SCALE`]); a draw
/// of `0x7fff` (side 4) keeps the place the last draw left in `last` (the caller's across its
/// calls; none yet is [`STACK_PLACE`]). A place on the map (`COMISS`: a NaN coordinate passes)
/// in `zone` (`zone_ok`, the caller's: any for −1) that the unit's master may take and the unit
/// could stand on (`free`, the caller's) is the drop location.
///
/// # Errors
///
/// [`STACK_PLACE`], and what the caller's tests give.
#[allow(clippy::too_many_arguments)]
pub fn find_drop_location(
    around: Around,
    unit_radius: [f32; 2],
    map: [i32; 2],
    last: &mut Option<[f32; 2]>,
    rng: &mut Well512,
    zone_ok: &mut dyn FnMut([f32; 2]) -> Result<bool, GenieError>,
    free: &mut dyn FnMut([f32; 2]) -> Result<bool, GenieError>,
) -> Result<Option<[f32; 2]>, GenieError> {
    let [cx, cy] = around.center;
    let [rx, ry] = around.radius;
    let off_x = rx + (unit_radius[0] + DROP_GAP);
    let off_y = ry + (unit_radius[1] + DROP_GAP);
    let along = |r: u32, radius: f32, centre: f32| {
        ((r as i32 as f32 * (radius + radius)) / DRAW_SCALE - radius) + centre
    };
    let (w, h) = (map[0] as f32, map[1] as f32);
    for _ in 0..DROP_TRIES {
        let side = (rng.rand15() as i32 * 4) / 0x7fff;
        let place = match side {
            0 => [cx + off_x, along(rng.rand15(), ry, cy)],
            1 => [cx - off_x, along(rng.rand15(), ry, cy)],
            2 => {
                let y = cy + off_y;
                [along(rng.rand15(), rx, cx), y]
            }
            3 => {
                let y = cy - off_y;
                [along(rng.rand15(), rx, cx), y]
            }
            _ => last.ok_or(STACK_PLACE)?,
        };
        *last = Some(place);
        let [x, y] = place;
        if 0.0 > x || 0.0 > y || x >= w || y >= h {
            continue;
        }
        if zone_ok(place)? && free(place)? {
            return Ok(Some(place));
        }
    }
    Ok(None)
}

/// The map as a unit's Transport reads it ([`Shore`]): the map's terrain, the zone maps and the
/// planners, the unit's terrain restriction (its master's `+0x606`; none below 0) and its tile.
pub(crate) struct MapShore<'a> {
    terrain: &'a crate::terrain::Terrain,
    zones: &'a mut crate::zones::Zones,
    pathing: &'a mut crate::pathing::Pathing,
    tiles: &'a crate::spatial::Tiles,
    map: [i32; 2],
    restriction: Option<usize>,
    tile: [i32; 2],
}

impl<'a> MapShore<'a> {
    /// The map about a unit of terrain restriction `restriction` at `position`.
    ///
    /// # Errors
    ///
    /// A map without its terrain.
    pub(crate) fn new(
        spatial: &'a crate::spatial::Spatial,
        zones: &'a mut crate::zones::Zones,
        pathing: &'a mut crate::pathing::Pathing,
        restriction: i16,
        position: [f32; 3],
    ) -> Result<Self, GenieError> {
        let terrain = spatial.terrain.as_ref().ok_or(crate::zones::NO_TERRAIN)?;
        Ok(Self {
            terrain,
            zones,
            pathing,
            tiles: spatial.tiles(),
            map: spatial.map,
            restriction: usize::try_from(restriction).ok(),
            tile: [cvttss2si(position[0]), cvttss2si(position[1])],
        })
    }

    /// The unit's zone map (`FUN_1411931c0`: the map's for its restriction, `FUN_1414138d0`).
    fn zone_map(&mut self) -> Result<Option<&crate::zones::ZoneMap>, GenieError> {
        let Some(r) = self.restriction else {
            return Ok(None);
        };
        let values = self
            .terrain
            .restrictions
            .get(r)
            .ok_or(crate::zones::UNKNOWN_RESTRICTION)?;
        let [w, h] = self.map;
        Ok(Some(self.zones.of(w, h, &self.terrain.kinds, values)))
    }
}

impl Shore for MapShore<'_> {
    fn zone(&mut self, x: i32, y: i32) -> Result<u16, GenieError> {
        if x < 0 || y < 0 {
            return Ok(0);
        }
        Ok(self.zone_map()?.map_or(0, |m| m.zone(x, y)))
    }

    fn near(&mut self, zone: u8, reach: f32) -> Result<bool, GenieError> {
        let tile = self.tile;
        Ok(self
            .zone_map()?
            .is_some_and(|m| m.near(u16::from(zone), tile, reach).is_some()))
    }

    fn flags(&self, x: i16, y: i16) -> u8 {
        self.terrain.flags_at(x, y)
    }

    fn shore_point(&mut self, from: [f32; 2], to: [f32; 2]) -> Option<[f32; 2]> {
        let planner = self.pathing.planner(self.restriction?, self.tiles)?;
        planner.last_open_point(from, to)
    }
}

/// The transports' parts that reach the world: the unload command's install, the unload and the
/// units put ashore.
impl crate::turn::Turns {
    /// What the Transport of player `p`'s object at `i` reads of it ([`Ship`]): its place, its
    /// master's radii, whether an object is inside it, the map's size and whether it has speed.
    fn ship(&self, p: usize, i: usize) -> Result<Ship, GenieError> {
        let unit = &self.players[p].objects[i];
        let master = self.master_of(p, unit.life.master_id)?;
        Ok(Ship {
            position: unit.position,
            radius: master.radius,
            loaded: !unit.garrison.is_empty(),
            map: self.spatial.map,
            moves: unit.mover.as_ref().is_some_and(|m| m.stats.speed != 0.0),
        })
    }

    /// The unit's unload command (slot 0x3a8, `FUN_14108d050`) at `to` for player `p`'s object at
    /// `i`, after the unit AI that gave it: a Transport on top (type 12, `FUN_140a7be70`) is told
    /// to work at the unit's own place ("Telling existing action to work": slot 9 with the unit
    /// itself as its object, [`TransportAction::work`]); else the actions go (`FUN_140a98620`,
    /// [`crate::turn::Turns::clear_actions`]) and a Transport at the unit's place
    /// ([`TransportAction::new`]) replaces them (slot 0x808: its slot 0x70, the base's, does
    /// nothing; the list's add starts it, [`TransportAction::start`]), then takes the work order
    /// at `to` from the unit itself.
    ///
    /// # Errors
    ///
    /// A map without its terrain, and what the actions' teardowns do not model.
    pub(crate) fn transport_order(
        &mut self,
        p: usize,
        i: usize,
        to: [f32; 3],
    ) -> Result<(), GenieError> {
        let existing = self.players[p].objects[i].top_action_kind() == Some(TRANSPORT);
        if !existing {
            self.clear_actions(p, i)?;
        }
        let ship = self.ship(p, i)?;
        let restriction = self
            .master_of(p, self.players[p].objects[i].life.master_id)?
            .terrain_restriction;
        let mut shore = MapShore::new(
            &self.spatial,
            &mut self.zones,
            &mut self.pathing,
            restriction,
            ship.position,
        )?;
        let unit = &mut self.players[p].objects[i];
        let changes = if let Some(t) = unit.transport.as_mut().filter(|_| existing) {
            vec![t.work(ship.position, true, &ship, &mut shore)?]
        } else {
            let mut t = TransportAction::new(ship.position);
            let started = t.start(true, &ship, &mut shore);
            let worked = t.work(to, true, &ship, &mut shore)?;
            unit.transport = Some(t);
            vec![started, worked]
        };
        for c in changes {
            self.transport_changed(p, i, c)?;
        }
        Ok(())
    }

    /// The Transport of player `p`'s object at `i` takes a move to `point` (its slot 8,
    /// [`crate::turn::Unit::transport_sail`]).
    ///
    /// # Errors
    ///
    /// A map without its terrain, a unit of a master the slice does not have.
    pub(crate) fn transport_move(
        &mut self,
        p: usize,
        i: usize,
        point: [f32; 3],
    ) -> Result<(), GenieError> {
        let rng = self.streams.get(crate::rng::Stream::Sprite);
        self.players[p].objects[i].transport_sail(
            point,
            (&self.spatial, &mut self.zones, &mut self.pathing),
            (self.masters.get(p), &self.graphics, rng),
        )
    }

    /// What a state change of the Transport of player `p`'s object at `i` leaves ([`Changed`]):
    /// the unit stops (its `MoveTo`'s teardown, slot 0x5d0), then takes its standing sprite (slot
    /// 0x68, [`crate::turn::Unit::show_standing`]).
    fn transport_changed(&mut self, p: usize, i: usize, c: Changed) -> Result<(), GenieError> {
        let unit = &mut self.players[p].objects[i];
        if c.stopped
            && let Some(m) = unit.mover.as_mut()
        {
            m.stop()?;
        }
        if c.stands {
            let rng = self.streams.get(crate::rng::Stream::Sprite);
            unit.show_standing(self.masters.get(p), &self.graphics, rng)?;
        }
        Ok(())
    }

    /// The unload of the Transport of player `p`'s object at `i` in its update's state 6
    /// (`FUN_140aa3ba0`, [`TransportStep::unload`]): the units inside go ashore in the zone of
    /// the point's tile ([`Shore::zone`]; any zone without a point) ([`Turns::unload_all`]).
    /// With none ashore the point moves ([`retry_point`], the Action stream) and the Transport
    /// sails there to unload (`set_state(4)`, then `+0x68`) when a draw found land, else the
    /// owner hears 0x207 (no unload there); with some ashore and more inside the owner hears
    /// 0x206 (its slot 0x2e8 with the transport's action value: the interface's messages, the
    /// old pather's hulled area, a computer player's AI, [`crate::unit_ai::alert_owner`]). Then
    /// the update's end ([`TransportAction::unloaded`]).
    ///
    /// # Errors
    ///
    /// A map without its terrain, a computer player's AI hearing it, and what the units put
    /// ashore do not model.
    pub(crate) fn transport_unload(&mut self, p: usize, i: usize) -> Result<(), GenieError> {
        let unit = &self.players[p].objects[i];
        let t = unit
            .transport
            .ok_or(GenieError::unreachable("an unload without its Transport"))?;
        let restriction = self.master_of(p, unit.life.master_id)?.terrain_restriction;
        let [x, y, _] = t.point;
        let zone = if x == NONE {
            u32::MAX
        } else {
            let mut shore = MapShore::new(
                &self.spatial,
                &mut self.zones,
                &mut self.pathing,
                restriction,
                unit.position,
            )?;
            u32::from(shore.zone(cvttss2si(x), cvttss2si(y))?)
        };
        let count = self.unload_all(p, i, zone)?;
        let computer = self.players[p].computer;
        let ship = self.ship(p, i)?;
        let mut changes = Vec::new();
        if count == 0 {
            let terrain = self
                .spatial
                .terrain
                .as_ref()
                .ok_or(crate::zones::NO_TERRAIN)?;
            let rng = self.streams.get(crate::rng::Stream::Action);
            let (point, sails) = retry_point([x, y], ship.map, |tx, ty| terrain.read(tx, ty), rng);
            let mut shore = MapShore::new(
                &self.spatial,
                &mut self.zones,
                &mut self.pathing,
                restriction,
                ship.position,
            )?;
            let t = self.players[p].objects[i]
                .transport
                .as_mut()
                .expect("read above");
            t.point[0] = point[0];
            t.point[1] = point[1];
            if sails {
                changes.push(t.set_state(SAIL_TO_UNLOAD, &ship, &mut shore));
                t.retarget = true;
            } else {
                crate::unit_ai::alert_owner(computer)?;
            }
        } else if ship.loaded {
            crate::unit_ai::alert_owner(computer)?;
        }
        let mut shore = MapShore::new(
            &self.spatial,
            &mut self.zones,
            &mut self.pathing,
            restriction,
            ship.position,
        )?;
        let t = self.players[p].objects[i]
            .transport
            .as_mut()
            .expect("read above");
        changes.push(t.unloaded(&ship, &mut shore));
        for c in changes {
            self.transport_changed(p, i, c)?;
        }
        Ok(())
    }

    /// `FUN_141191b40`: the objects inside the transport at (`p`, `i`) go ashore in `zone` (−1
    /// for any), from the last in: each where a drop location is free ([`Turns::drop_location`])
    /// about the last spot one took (the first: the transport's place and radii), else about the
    /// spot before, and so on back; one that finds none stays inside, and so does every one
    /// after it (the spots are used up). A unit put ashore ([`Turns::come_ashore`]) makes its
    /// place, with its master's radii, the next spot (up to 29). Returns how many went ashore,
    /// a byte.
    fn unload_all(&mut self, p: usize, i: usize, zone: u32) -> Result<u8, GenieError> {
        const SPOTS: usize = 0x1d;
        let ship = &self.players[p].objects[i];
        let radius = self.master_of(p, ship.life.master_id)?.radius;
        let mut spots = [Around {
            center: [0.0; 2],
            radius: [0.0; 2],
        }; SPOTS + 1];
        spots[1] = Around {
            center: [ship.position[0], ship.position[1]],
            radius,
        };
        let mut k = 1;
        let mut count = 0u8;
        // the drop locations' last place, across the units (FUN_141191b40's locals)
        let mut last = None;
        let mut j = ship.garrison.len();
        while 0 < j {
            j -= 1;
            let Some(&id) = self.players[p].objects[i].garrison.get(j) else {
                continue;
            };
            // the game skips a null entry; an id the world no longer holds is one
            let Some((up, u)) = self.object(id) else {
                continue;
            };
            let made = self.master_of(up, u.life.master_id)?.clone();
            while 0 < k {
                if let Some(spot) =
                    self.drop_location((p, i), id, &made, spots[k], zone, &mut last)?
                {
                    self.come_ashore((p, i), id, &made, spot)?;
                    count = count.wrapping_add(1);
                    if k < SPOTS {
                        k += 1;
                        spots[k] = Around {
                            center: spot,
                            radius: made.radius,
                        };
                    }
                    break;
                }
                k -= 1;
            }
        }
        Ok(count)
    }

    /// `FUN_141192d10` for the unit `id` (of master `made`) about `around`, in `zone` of the
    /// transport at (`p`, `i`) ([`find_drop_location`] on the Object stream, the place `last`
    /// across the caller's units). A place is in the zone when the zone of its tile on the
    /// transport's zone map (`FUN_1411958c0` on its `CVTTSS2SI` tile, 0 below 0 or without a map),
    /// cut to its low byte, is it, or touches it (`FUN_141414f70` on both low bytes; without a
    /// map none does); −1 takes any. A place is free where the unit's master may stand for the
    /// transport's owner (its slot 0x50, [`crate::spawn::placement`] with the spawn's arguments:
    /// its arguments 9 and 12, 1 and 0 here, change nothing the slice reads) and the unit could
    /// stand (its slot 0x278, [`crate::spatial::Spatial::point_clear`] by its radius and its
    /// restriction's mask).
    pub(crate) fn drop_location(
        &mut self,
        (p, i): (usize, usize),
        id: i32,
        made: &crate::masters::Master,
        around: Around,
        zone: u32,
        last: &mut Option<[f32; 2]>,
    ) -> Result<Option<[f32; 2]>, GenieError> {
        let ship = &self.players[p].objects[i];
        let restriction = self.master_of(p, ship.life.master_id)?.terrain_restriction;
        let terrain = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(crate::zones::NO_TERRAIN)?;
        let [w, h] = self.spatial.map;
        let zone_map = match usize::try_from(restriction) {
            Ok(r) if zone != u32::MAX => {
                let values = terrain
                    .restrictions
                    .get(r)
                    .ok_or(crate::zones::UNKNOWN_RESTRICTION)?;
                Some(self.zones.find(w, h, &terrain.kinds, values, false))
            }
            _ => None,
        };
        let moving = made.moving.as_ref().ok_or(crate::spawn::NOT_MOVING)?;
        let terrain = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(crate::zones::NO_TERRAIN)?;
        let values = terrain
            .restrictions
            .get(moving.restriction)
            .ok_or(crate::zones::UNKNOWN_RESTRICTION)?;
        let placing = crate::spawn::Placing {
            player: p,
            radius: made.radius,
            tall: made.tall,
            class: made.class,
            restriction: values,
            hill_mode: made.hill_mode,
            placement: made.placement,
        };
        let unit = self.object(id).map(|(_, u)| u);
        let (grow, mask) = unit.and_then(|u| u.mover.as_ref()).map_or_else(
            || (made.radius[0], terrain.restriction_mask(moving.restriction)),
            |m| (m.stats.radius[0], m.stats.restriction_mask),
        );
        let zone_map = zone_map.map(|k| self.zones.map(k));
        let mut rng = self.streams.get(crate::rng::Stream::Object).clone();
        let map = crate::spawn::WorldMap { turns: self };
        let spatial = &self.spatial;
        let found = find_drop_location(
            around,
            made.radius,
            [w, h],
            last,
            &mut rng,
            &mut |[x, y]| {
                if zone == u32::MAX {
                    return Ok(true);
                }
                let (tx, ty) = (cvttss2si(x), cvttss2si(y));
                let tile_zone = match zone_map {
                    Some(m) if 0 <= tx && 0 <= ty => m.zone(tx, ty),
                    _ => 0,
                };
                let low = tile_zone & 0xff;
                if u32::from(low) == zone {
                    return Ok(true);
                }
                let Some(m) = zone_map else {
                    return Ok(false);
                };
                m.adjacent(low, (zone & 0xff) as u16)
                    .ok_or(GenieError::unreachable("a zone adjacency past its table"))
            },
            &mut |[x, y]| {
                let [rx, ry] = made.radius;
                spatial.check_known([x - rx, y - ry, x + rx, y + ry])?;
                if crate::spawn::placement(&placing, &map, [x, y], false)?
                    != crate::spawn::Tested::Free
                {
                    return Ok(false);
                }
                spatial.point_clear(id, [x, y], grow, mask)
            },
        );
        *self.streams.get(crate::rng::Stream::Object) = rng;
        found
    }

    /// `FUN_141191b40`'s put ashore of the unit `id` (of master `made`) from the transport at
    /// (`p`, `i`) at `spot`: it takes the transport's place (its place's fields), leaves it (slot
    /// 0x1a8) for the spot at the transport's height (slot 0x58, [`Turns::leave_container`]); its
    /// `+0x688`, its box (`+0x6fc`..`+0x704`) and its sprites' places (`FUN_141189d50`), the
    /// interface's, are not held; its unit AI's state position (`+0x1ac`, `+0x1b0`) becomes the
    /// spot. A siege tower's (class 13) unload is not modelled.
    fn come_ashore(
        &mut self,
        (p, i): (usize, usize),
        id: i32,
        made: &crate::masters::Master,
        spot: [f32; 2],
    ) -> Result<(), GenieError> {
        let ship = &self.players[p].objects[i];
        let at = ship.position;
        if self.master_of(p, ship.life.master_id)?.class == crate::garrison::SIEGE_CLASS {
            return Err(crate::garrison::SIEGE_UNLOAD);
        }
        let (up, ui) = self.live_inside(id)?;
        self.players[up].objects[ui].position = at;
        self.leave_container((p, i), (up, ui), made, [spot[0], spot[1], at[2]])?;
        if let Some(ai) = self.players[up].objects[ui].ai.as_mut() {
            ai.state_position = spot;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map of zones by column (the tile's x over 10) and terrain flags by a closure; the shore
    /// point the planner gives, recorded.
    struct Coast {
        flags: fn(i16, i16) -> u8,
        near: bool,
        shore: Option<[f32; 2]>,
        asked: Vec<([f32; 2], [f32; 2])>,
    }

    impl Shore for Coast {
        fn zone(&mut self, x: i32, _y: i32) -> Result<u16, GenieError> {
            Ok(if x < 0 { 0 } else { (x / 10) as u16 + 1 })
        }

        fn near(&mut self, _zone: u8, _reach: f32) -> Result<bool, GenieError> {
            Ok(self.near)
        }

        fn flags(&self, x: i16, y: i16) -> u8 {
            (self.flags)(x, y)
        }

        fn shore_point(&mut self, from: [f32; 2], to: [f32; 2]) -> Option<[f32; 2]> {
            self.asked.push((from, to));
            self.shore
        }
    }

    fn coast(near: bool) -> Coast {
        Coast {
            flags: |x, _| if 20 <= x { LAND_FLAGS } else { 1 },
            near,
            shore: None,
            asked: Vec::new(),
        }
    }

    /// One update as the unit's runs it: the `MoveTo` (none walking here), then the state.
    fn run(t: &mut TransportAction, s: &Ship, c: &mut Coast) -> Result<TransportStep, GenieError> {
        let walked = t.walk(None, PathAsk::default())?;
        t.update(walked, s, c)
    }

    fn ship(loaded: bool) -> Ship {
        Ship {
            position: [12.5, 8.5, 0.0],
            radius: [0.5, 0.5],
            loaded,
            map: [40, 30],
            moves: true,
        }
    }

    #[test]
    fn a_new_transport_sails_to_its_own_place_and_a_start_off_the_map_idles() {
        let s = ship(true);
        let mut c = coast(true);
        let mut t = TransportAction::new(s.position);
        assert_eq!((t.state, t.mode, t.retarget), (0, -1, false));
        // the list's add (first_in_stack with 1): set_state(11), its MoveTo to the shore point
        // on the way, none here, so the place itself
        let changed = t.start(true, &s, &mut c);
        assert_eq!(changed, Changed::default());
        assert_eq!((t.state, t.move_to), (SAIL, Some(MoveTo::Start)));
        assert_eq!(c.asked, [([12.5, 8.5], [12.5, 8.5])]);
        // a NaN point sails too (COMISS/JA, JBE)
        let mut nan = TransportAction::new([f32::NAN, 1.0, 0.0]);
        let _ = nan.start(true, &s, &mut c);
        assert_eq!(nan.state, SAIL);
        // below 0, or not first: idle with the standing sprite
        let mut off = TransportAction::new([3.0, -1.0, 0.0]);
        assert!(off.start(true, &s, &mut c).stands);
        assert_eq!((off.state, off.move_to), (IDLE, None));
        let mut later = TransportAction::new(s.position);
        let _ = later.start(false, &s, &mut c);
        assert_eq!(later.state, IDLE);
    }

    #[test]
    fn a_sail_takes_the_shore_point_and_drops_its_old_move_to() {
        let s = ship(true);
        let mut c = coast(true);
        c.shore = Some([19.5, 8.5]);
        let mut t = TransportAction::new(s.position);
        let _ = t.start(true, &s, &mut c);
        t.point = [25.0, 8.0, 0.0];
        // set_state(4) again: the old MoveTo's teardown stops the unit, the new one starts
        let changed = t.set_state(SAIL_TO_UNLOAD, &s, &mut c);
        assert!(changed.stopped && !changed.stands);
        assert_eq!(t.point, [19.5, 8.5, 0.0]);
        assert_eq!(t.move_to, Some(MoveTo::Start));
        assert_eq!(c.asked[1], ([12.5, 8.5], [25.0, 8.0]));
        // a unit without speed: its quiet MoveTo fails at its start
        let still = Ship { moves: false, ..s };
        let _ = t.set_state(SAIL, &still, &mut c);
        assert_eq!(t.move_to, Some(MoveTo::Failed));
    }

    #[test]
    fn a_move_sails_to_its_point_and_a_stop_idles() {
        let s = ship(true);
        let mut c = coast(true);
        c.shore = Some([19.5, 8.5]);
        let mut t = TransportAction {
            state: UNLOAD,
            mode: 1,
            ..TransportAction::new(s.position)
        };
        let changed = t.move_to([25.0, 8.0, 2.0], &s, &mut c);
        assert_eq!(changed, Changed::default());
        assert_eq!(
            (t.state, t.mode, t.point, t.move_to),
            (SAIL, 0, [19.5, 8.5, 2.0], Some(MoveTo::Start))
        );
        assert_eq!(c.asked, [([12.5, 8.5], [25.0, 8.0])]);
        let stopped = t.stop(&s, &mut c);
        assert!(stopped.stopped && stopped.stands);
        assert_eq!((t.state, t.move_to), (IDLE, None));
    }

    #[test]
    fn a_work_order_near_its_zone_unloads_at_once_else_sails_there() {
        let s = ship(true);
        // within reach of the point's zone and 2 from the footprint (squared 4, at most 4)
        let mut c = coast(true);
        let mut t = TransportAction::new(s.position);
        let _ = t.work([15.0, 8.5, 1.0], true, &s, &mut c).unwrap();
        assert_eq!((t.state, t.mode, t.point), (UNLOAD, 1, [15.0, 8.5, 1.0]));
        // a bit further: sail there to unload, from the unit itself, else just sail
        let mut t = TransportAction::new(s.position);
        let _ = t.work([15.1, 8.5, 1.0], true, &s, &mut c).unwrap();
        assert_eq!((t.state, t.point), (SAIL_TO_UNLOAD, [15.1, 8.5, 1.0]));
        let mut t = TransportAction::new(s.position);
        let _ = t.work([15.1, 8.5, 1.0], false, &s, &mut c).unwrap();
        assert_eq!(t.state, SAIL);
        // not near the zone: sail
        let mut far = coast(false);
        let mut t = TransportAction::new(s.position);
        let _ = t.work([13.0, 8.5, 0.0], true, &s, &mut far).unwrap();
        assert_eq!(t.state, SAIL_TO_UNLOAD);
        // a point below 0 or NaN far away changes nothing but +0x34
        for bad in [[-1.0, 3.0, 0.0], [3.0, f32::NAN, 0.0]] {
            let mut t = TransportAction::new(s.position);
            assert_eq!(t.work(bad, true, &s, &mut far).unwrap(), Changed::default());
            assert_eq!((t.state, t.mode, t.point), (0, 1, s.position));
        }
        // a NaN gap stays NaN (MAXSS) and is not within (COMISS/JC): it sails
        let mut t = TransportAction::new(s.position);
        let nan_ship = Ship {
            radius: [f32::NAN, 0.5],
            ..s
        };
        let _ = t.work([12.5, 8.5, 0.0], true, &nan_ship, &mut c).unwrap();
        assert_eq!(t.state, SAIL_TO_UNLOAD);
    }

    #[test]
    fn the_shore_search_looks_about_the_footprint_for_the_point_s_zone_or_land() {
        let mut c = coast(true);
        // the ship at x 12.5 with radius 0.5: tiles 10..=15; zone 2 is x 10..19
        let mut t = TransportAction::new(ship(true).position);
        t.point = [17.0, 3.0, 0.0];
        assert!(t.next_to_shore(&ship(true), &mut c).unwrap());
        // zone 3 (x 20..29) lies past x 15
        t.point = [25.0, 3.0, 0.0];
        assert!(!t.next_to_shore(&ship(true), &mut c).unwrap());
        // without a point: land (x 20 and up) is past x 15 too, but not from x 17.5
        t.point = [NONE, 0.0, 0.0];
        assert!(!t.next_to_shore(&ship(true), &mut c).unwrap());
        let east = Ship {
            position: [17.5, 8.5, 0.0],
            ..ship(true)
        };
        assert!(t.next_to_shore(&east, &mut c).unwrap());
        // the box stays on the map: a ship at the west edge reads no tile below 0
        let mut c = Coast {
            flags: |x, y| {
                assert!(0 <= x && 0 <= y, "({x}, {y})");
                0
            },
            ..coast(true)
        };
        let west = Ship {
            position: [0.5, 0.5, 0.0],
            ..ship(true)
        };
        assert!(!t.next_to_shore(&west, &mut c).unwrap());
    }

    #[test]
    fn an_unload_out_of_reach_or_beside_no_shore_tells_the_owner_unless_over_water() {
        let s = ship(true);
        let mut t = TransportAction::new(s.position);
        t.state = UNLOAD;
        t.point = [25.0, 3.0, 0.0];
        t.retarget = true;
        // not near the point's zone: the owner hears it, +0x68 clears, the action idles
        let step = run(&mut t, &s, &mut coast(false)).unwrap();
        assert!(step.cannot_unload && !step.unload && step.changed.stands);
        assert_eq!((t.state, t.retarget, step.code), (IDLE, false, 0));
        // near, but zone 3 is not beside the footprint: over land at the point it tells the
        // owner, over water it waits silently
        let mut t = TransportAction {
            state: UNLOAD,
            point: [25.0, 3.0, 0.0],
            ..TransportAction::new(s.position)
        };
        let step = run(&mut t, &s, &mut coast(true)).unwrap();
        assert!(step.cannot_unload);
        let mut wet = coast(true);
        wet.flags = |_, _| 1;
        let mut t = TransportAction {
            state: UNLOAD,
            point: [25.0, 3.0, 0.0],
            ..TransportAction::new(s.position)
        };
        let step = run(&mut t, &s, &mut wet).unwrap();
        assert!(!step.cannot_unload && step.changed.stands);
        // empty: nothing to unload, the action idles
        let mut t = TransportAction {
            state: UNLOAD,
            point: [25.0, 3.0, 0.0],
            ..TransportAction::new(s.position)
        };
        let step = run(&mut t, &ship(false), &mut coast(true)).unwrap();
        assert!(!step.cannot_unload && !step.unload);
        assert_eq!(t.state, IDLE);
    }

    #[test]
    fn an_unload_beside_the_shore_is_the_caller_s_then_idles_unless_it_sails_on() {
        let s = ship(true);
        let mut c = coast(true);
        let mut t = TransportAction {
            state: UNLOAD,
            point: [17.0, 3.0, 0.0],
            ..TransportAction::new(s.position)
        };
        let step = run(&mut t, &s, &mut c).unwrap();
        assert!(step.unload && !step.cannot_unload);
        assert_eq!(t.state, UNLOAD);
        assert!(t.unloaded(&s, &mut c).stands);
        assert_eq!(t.state, IDLE);
        // a retarget's set_state(4) stays
        t.state = SAIL_TO_UNLOAD;
        t.retarget = true;
        assert_eq!(t.unloaded(&s, &mut c), Changed::default());
        assert_eq!(t.state, SAIL_TO_UNLOAD);
    }

    #[test]
    fn a_sail_ends_with_its_move_to_and_a_failed_one_returns_3() {
        let s = ship(true);
        let mut c = coast(true);
        // state 4 without a MoveTo: the list answers 1, so it unloads
        let mut t = TransportAction {
            state: SAIL_TO_UNLOAD,
            ..TransportAction::new(s.position)
        };
        let step = run(&mut t, &s, &mut c).unwrap();
        assert_eq!((t.state, step.code), (UNLOAD, 0));
        // state 11 with an ended MoveTo: idle, the MoveTo gone (its teardown stops the unit)
        let mut t = TransportAction {
            state: SAIL,
            move_to: Some(MoveTo::Arrived),
            ..TransportAction::new(s.position)
        };
        let step = run(&mut t, &s, &mut c).unwrap();
        assert_eq!(t.state, IDLE);
        assert!(step.changed.stopped && step.changed.stands);
        // a failed MoveTo (code 3) ends a sail to unload too
        let mut t = TransportAction {
            state: SAIL_TO_UNLOAD,
            move_to: Some(MoveTo::Failed),
            ..TransportAction::new(s.position)
        };
        let _ = run(&mut t, &s, &mut c).unwrap();
        assert_eq!(t.state, UNLOAD);
        // state 13: the unit hears it failed, the action idles and returns 3
        let mut t = TransportAction {
            state: FAILED,
            ..TransportAction::new(s.position)
        };
        let step = run(&mut t, &s, &mut c).unwrap();
        assert!(step.failed);
        assert_eq!((t.state, step.code), (IDLE, 3));
        // state 1 is done; an idle one does nothing
        t.state = DONE;
        assert_eq!(run(&mut t, &s, &mut c).unwrap().code, 1);
        t.state = IDLE;
        assert_eq!(run(&mut t, &s, &mut c).unwrap(), TransportStep::default());
    }

    /// `FUN_140a992b0` as a Transport runs it: a start asks for its path to the point (the
    /// planner gives the walker its two paths), a move walks on; without the walk either is
    /// an error, and a waiting `MoveTo` (only a villager's waits) is not a ship's.
    #[test]
    fn a_sail_s_move_to_asks_for_its_path_then_walks_on() {
        use crate::movement::tests::WalkRig;
        let sailing = |move_to| TransportAction {
            move_to,
            point: [35.5, 88.5, 0.0],
            ..TransportAction::new([28.0, 89.0, 0.0])
        };
        let ask = PathAsk::default();
        let mut rig = WalkRig::on_the_path();
        rig.mover.paths.clear();
        let mut t = sailing(Some(MoveTo::Start));
        assert_eq!(t.walk(Some(rig.walk(SAIL)), ask), Ok(0));
        assert_eq!(t.move_to, Some(MoveTo::Moving));
        assert_eq!(rig.mover.paths.len(), 2);
        let mut rig = WalkRig::on_the_path();
        let mut t = sailing(Some(MoveTo::Moving));
        assert_eq!(t.walk(Some(rig.walk(SAIL)), ask), Ok(0));
        assert_eq!(t.move_to, Some(MoveTo::Moving));
        assert!(sailing(Some(MoveTo::Start)).walk(None, ask).is_err());
        assert!(sailing(Some(MoveTo::Moving)).walk(None, ask).is_err());
        let mut t = sailing(Some(MoveTo::Waiting));
        assert!(t.walk(Some(rig.walk(SAIL)), ask).is_err());
    }

    fn rng() -> Well512 {
        let mut r = Well512::default();
        r.set_seed(7);
        r
    }

    #[test]
    fn a_retry_draws_about_the_old_point_until_it_leaves_the_water() {
        // the draws as the game makes them: x then y, each r / 0xccc tiles
        let mut copy = rng();
        let draws: Vec<u32> = (0..6).map(|_| copy.rand15() / RETRY_STEP).collect();
        let point = [20.0, 20.0];
        let at = |i: usize| {
            [
                draws[2 * i] as f32 + 20.0 - 5.0,
                draws[2 * i + 1] as f32 + 20.0 - 5.0,
            ]
        };
        // land (terrain 0) everywhere: the first draw leaves the water
        let mut r = rng();
        assert_eq!(
            retry_point(point, [40, 40], |_, _| 0, &mut r),
            (at(0), true)
        );
        // water 22, 23 and 1 keep drawing: the third draw's tile is land
        let first = [at(0), at(1)].map(|p| [p[0] as i16, p[1] as i16]);
        let mut r = rng();
        let (found, sails) = retry_point(
            point,
            [40, 40],
            |x, y| match [x, y] {
                t if t == first[0] => 0x17,
                t if t == first[1] => 1,
                _ => 0,
            },
            &mut r,
        );
        assert_eq!((found, sails), (at(2), true));
        assert_eq!(r, copy);
        // off the map the last terrain stays: all water, 100 draws, the last one taken
        let mut r = rng();
        let (_, sails) = retry_point(point, [40, 40], |_, _| 0x16, &mut r);
        assert!(!sails);
        assert_eq!(r.draws, rng().draws + 200);
    }

    #[test]
    fn the_99th_draw_leaving_the_water_comes_too_late() {
        // water until the 99th draw: the loop breaks with no draws to spare
        let mut r = rng();
        let count = std::cell::Cell::new(0);
        let (_, sails) = retry_point(
            [20.0, 20.0],
            [400, 400],
            |_, _| {
                count.set(count.get() + 1);
                if count.get() < 99 { 0x16 } else { 0 }
            },
            &mut r,
        );
        assert!(!sails);
        // the 98th: one to spare
        let mut r = rng();
        count.set(0);
        let (_, sails) = retry_point(
            [20.0, 20.0],
            [400, 400],
            |_, _| {
                count.set(count.get() + 1);
                if count.get() < 98 { 0x16 } else { 0 }
            },
            &mut r,
        );
        assert!(sails);
    }

    #[test]
    fn a_drop_location_is_drawn_on_a_side_past_both_footprints() {
        let around = Around {
            center: [10.0, 10.0],
            radius: [1.0, 1.0],
        };
        // the first draw's side and place, as the game computes them
        let mut copy = rng();
        let (r1, r2) = (copy.rand15(), copy.rand15());
        let side = (r1 as i32 * 4) / 0x7fff;
        let along = ((r2 as f32 * 2.0) / 32767.0 - 1.0) + 10.0;
        let gap = 1.0 + (0.5 + DROP_GAP);
        let expected = match side {
            0 => [10.0 + gap, along],
            1 => [10.0 - gap, along],
            2 => [along, 10.0 + gap],
            _ => [along, 10.0 - gap],
        };
        let mut last = None;
        let mut r = rng();
        let found = find_drop_location(
            around,
            [0.5, 0.5],
            [40, 40],
            &mut last,
            &mut r,
            &mut |_| Ok(true),
            &mut |_| Ok(true),
        )
        .unwrap();
        assert_eq!(found, Some(expected));
        assert_eq!(last, Some(expected));
        assert_eq!(r, copy);
        // nothing free: 50 tries, the last place kept for the next call
        let mut r = rng();
        let mut tries = 0;
        let none = find_drop_location(
            around,
            [0.5, 0.5],
            [40, 40],
            &mut last,
            &mut r,
            &mut |_| Ok(true),
            &mut |_| {
                tries += 1;
                Ok(false)
            },
        )
        .unwrap();
        assert_eq!((none, tries), (None, 50));
        // off the map: neither test runs
        let mut r = rng();
        let edge = Around {
            center: [0.0, 0.0],
            radius: [5.0, 5.0],
        };
        let mut tested = 0;
        let _ = find_drop_location(
            edge,
            [0.5, 0.5],
            [4, 4],
            &mut None,
            &mut r,
            &mut |p| {
                tested += 1;
                assert!(0.0 <= p[0] && 0.0 <= p[1] && p[0] < 4.0 && p[1] < 4.0);
                Ok(true)
            },
            &mut |_| Ok(false),
        )
        .unwrap();
        assert!(tested < 50);
    }
}
