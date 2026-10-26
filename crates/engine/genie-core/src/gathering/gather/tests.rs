use super::*;

/// A forager's task: food (kind 17, berries), range 0.25, work value 1 (the 4v4's 31139).
fn task() -> HuntTask {
    HuntTask {
        id: 2,
        range: 0.25,
        resource: 17,
        sprites: [-1; 4],
        work_value: 1.0,
        productivity: -1,
        kind4: -1,
    }
}

/// The 4v4's berry bush 28222 (master 59, class 7) as the builder 31139 takes it in update
/// 4863.
fn bush() -> Resource {
    Resource {
        id: 28222,
        position: [23.5, 93.5, 0.0],
        class: 7,
        master: 59,
        player: 0,
        workers: 0,
        drop_site: false,
        gap: 1.2,
        radius: [0.5, 0.5],
    }
}

/// A villager's rate (work rate 1, capacity 10, no productivity attribute).
const fn rate() -> StopRate {
    StopRate {
        work_rate: 1.0,
        capacity: 10.0,
        productivity: 1.0,
    }
}

fn target(amount: f32) -> Victim {
    Victim {
        id: 28222,
        amount,
        state: 2,
        class: 7,
        master_id: 59,
        storage: 125.0,
        ..Victim::default()
    }
}

/// A farm of master 50 (class 49, radii 1.5) at MarvinBlue/426049433's 12557.
fn farm() -> Resource {
    Resource {
        id: 12557,
        position: [112.5, 173.5, 3.0],
        class: FARM_CLASS,
        master: 50,
        player: 1,
        workers: 0,
        drop_site: false,
        gap: 0.0,
        radius: [1.5, 1.5],
    }
}

#[test]
fn a_farm_s_stop_is_a_tile_centre_of_its_footprint_other_than_the_last() {
    let mut rng = Well512::default();
    rng.set_seed(29910);
    let mut last = [0.0; 2];
    for _ in 0..50 {
        let [x, y] = stop_point([112.5, 173.5, 3.0], [1.5, 1.5], last, &mut rng);
        assert!([111.5, 112.5, 113.5].contains(&x) && [172.5, 173.5, 174.5].contains(&y));
        assert_ne!([x, y], last);
        last = [x, y];
    }
    // no footprint: the farm's own tile, drawn ten times when it is the last stop's
    let mut rng = Well512::default();
    let at = stop_point([10.0, 20.0, 0.0], [0.0, 0.0], [10.5, 20.5], &mut rng);
    assert_eq!((at, rng.draws), ([10.5, 20.5], 20));
}

#[test]
fn under_the_farm_setting_a_farm_s_gather_goes_in_four_timed_stops() {
    let rate = StopRate {
        work_rate: 0.33,
        capacity: 10.0,
        productivity: 1.0,
    };
    let g = GatherAction::new(task(), &farm(), true, rate).unwrap();
    assert_eq!(
        (g.stops, g.time_per_stop, g.farm),
        (4, ((10.0f32 * 1.0) / (0.33 * 1.0)) / 4.0, [112.5, 173.5])
    );
    // without the setting, or on a fish trap, one stop
    let one = GatherAction::new(task(), &farm(), false, rate).unwrap();
    let trap = Resource {
        master: 199,
        ..farm()
    };
    let trap = GatherAction::new(task(), &trap, true, rate).unwrap();
    assert_eq!((one.stops, one.time_per_stop, trap.stops), (1, 0.0, 1));
}

/// Slot 0x38 (`FUN_140a8e1b0`, `set_state(2)`): the `MoveTo` goes and stops the unit, the
/// timer clears, the unit stands, the target and the drop site go (returned for the
/// caller's release) and the point is −1.
#[test]
fn a_stop_idles_the_gather_and_lets_its_targets_go() {
    let mut mover = crate::gathering::hunt::tests::walker();
    let mut g = GatherAction {
        state: WALK,
        target: 31,
        carry_to: 9,
        point: [5.0, 6.0, 0.0],
        timer: 2.0,
        move_to: Some(MoveTo::Moving),
        walk_to: Some([5.0, 6.0, 0.0]),
        ..GatherAction::new(task(), &farm(), false, rate()).unwrap()
    };
    let stand = crate::objects::sprite::Graphic {
        id: 4,
        frames: 1,
        facets: 1,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 0,
    };
    let (mut sprite, mut rng) = (None, Well512::default());
    let released = g
        .halt(Some(&mut mover), Some(stand), &mut sprite, &mut rng)
        .unwrap();
    assert_eq!(released, [31, 9]);
    assert_eq!(
        (
            g.state, g.timer, g.target, g.carry_to, g.point, g.move_to, g.walk_to
        ),
        (IDLE, 0.0, -1, -1, [-1.0; 3], None, None)
    );
    assert_eq!(sprite.map(|s| s.graphic.id), Some(4));
    assert_eq!(mover.velocity.unwrap().velocity[2], 0.0);
}

#[test]
fn past_its_time_per_stop_a_farmer_walks_to_the_next_stop() {
    let mut mover = crate::gathering::hunt::tests::walker();
    let mut g = GatherAction {
        state: WORK,
        time_per_stop: 1.0,
        time_at_stop: 0.95,
        ..GatherAction::new(task(), &farm(), true, rate()).unwrap()
    };
    let mut rng = Well512::default();
    let at = stop_point([112.5, 173.5, 3.0], [1.5, 1.5], [0.0; 2], &mut rng.clone());
    g.set_walk([112.5, 173.5, 3.0], [1.5, 1.5], 0.0, &mut mover, &mut rng)
        .unwrap();
    assert_eq!(
        (g.state, g.last_stop, g.walk_to, g.move_to),
        (WALK, at, Some([at[0], at[1], 3.0]), Some(MoveTo::Start))
    );
}

#[test]
fn a_farm_s_food_sold_at_once_goes_to_the_player_and_the_gathering_goes_on() {
    let g = GatherAction {
        state: WORK,
        ..GatherAction::new(task(), &farm(), false, rate()).unwrap()
    };
    let (mut kind, mut held) = (FARM_FOOD_KIND, 0.0f32);
    let mut sprite = None;
    let (mut rng, mut action) = (Well512::default(), Well512::default());
    let mut gatherer = Gatherer {
        held_kind: &mut kind,
        held: &mut held,
        work_rate: 0.5,
        capacity: 10.0,
        productivity: 1.0,
        sprites: HuntSprites::default(),
        sprite: &mut sprite,
        sprite_rng: &mut rng,
        class: 4,
        master: 83,
        stance: 0,
        world_time: 0,
        farm_stops: false,
        posted: Vec::new(),
        action_rng: &mut action,
        player: 1,
        computer: false,
        stop_productivity: 1.0,
        food_sold: 1.0,
        sold: 0.0,
        anim_stopped: false,
    };
    let mut field = Victim {
        class: FARM_CLASS,
        master_id: 50,
        ..target(15.0)
    };
    assert_eq!(g.gather(&mut field, &mut gatherer, 0.5), Next::Stay);
    assert_eq!(
        (gatherer.sold, *gatherer.held, field.amount),
        (0.25, 0.0, 14.75)
    );
}

#[test]
fn a_gatherer_moved_walks_to_the_point_or_carries_its_load_to_the_drop_site() {
    let mut mover = crate::gathering::hunt::tests::walker();
    let gatherer = || GatherAction {
        state: WORK,
        ..GatherAction::new(task(), &bush(), true, rate()).unwrap()
    };
    // a move to a point (or an object that is no drop site): state 15 to the point
    let mut g = gatherer();
    g.take_move(None, [30.0, 40.0, 0.0], 7, 5.0, &mut mover)
        .unwrap();
    assert_eq!(
        (g.state, g.target, g.point, g.walk_to, g.move_to),
        (
            MOVED,
            -1,
            [30.0, 40.0, 0.0],
            Some([30.0, 40.0, 0.0]),
            Some(MoveTo::Start)
        )
    );
    // a loaded gatherer moved to its own mill carries the load there
    let mill = MoveTarget {
        id: 500,
        player: 7,
        drop_site: true,
        gap: 3.0,
    };
    let mut g = gatherer();
    g.take_move(Some(mill), [1.0; 3], 7, 5.0, &mut mover)
        .unwrap();
    assert_eq!(
        (g.state, g.target, g.carry_to, g.point),
        (DROP, -1, 500, [-1.0; 3])
    );
    // already on its way there, nothing changes; empty-handed it walks to the point
    let walking = g.move_to;
    g.take_move(Some(mill), [1.0; 3], 7, 5.0, &mut mover)
        .unwrap();
    assert_eq!((g.state, g.move_to), (DROP, walking));
    let mut g = gatherer();
    g.take_move(Some(mill), [1.0; 3], 7, 0.0, &mut mover)
        .unwrap();
    assert_eq!((g.state, g.carry_to), (MOVED, -1));
}

#[test]
fn at_its_point_a_human_s_moved_gatherer_idles_and_a_computer_s_is_done() {
    let run = |computer: bool| {
        let mut mover = crate::gathering::hunt::tests::walker();
        let mut g = GatherAction {
            state: MOVED,
            target: -1,
            move_to: Some(MoveTo::Arrived),
            walk_to: Some([30.0, 40.0, 0.0]),
            ..GatherAction::new(task(), &bush(), true, rate()).unwrap()
        };
        let mut world = crate::map::spatial::Spatial::default();
        let walk = Walk {
            mover: Some(&mut mover),
            me: crate::movement::Me {
                id: 1,
                position: [30.0, 40.0, 0.0],
                group: -1,
                action_state: MOVED,
            },
            dt: 0.1,
            world: &mut world,
            plan: None,
        };
        let (mut kind, mut held) = (17i16, 0.0f32);
        let mut sprite = None;
        let (mut rng, mut action) = (Well512::default(), Well512::default());
        let mut gatherer = Gatherer {
            held_kind: &mut kind,
            held: &mut held,
            work_rate: 0.31,
            capacity: 10.0,
            productivity: 1.0,
            sprites: HuntSprites::default(),
            sprite: &mut sprite,
            sprite_rng: &mut rng,
            class: 4,
            master: 83,
            stance: 0,
            world_time: 0,
            farm_stops: true,
            posted: Vec::new(),
            action_rng: &mut action,
            player: 7,
            computer,
            stop_productivity: 1.0,
            food_sold: 0.0,
            sold: 0.0,
            anim_stopped: false,
        };
        assert_eq!(g.update_moved(walk, &mut gatherer), Ok(Next::Stay));
        (g.state, g.point, g.move_to)
    };
    assert_eq!(run(false), (IDLE, [-1.0; 3], None));
    assert_eq!(run(true).0, DONE);
}

#[test]
fn a_gatherer_without_a_target_or_a_load_asks_for_another_every_ten_seconds() {
    // the 4v4's forager 31237 at update 10820: its bush went while it walked there
    let run = |master: i16, g: &mut GatherAction, dt: f32| {
        let mut mover = crate::gathering::hunt::tests::walker();
        let mut world = crate::map::spatial::Spatial::default();
        let walk = Walk {
            mover: Some(&mut mover),
            me: crate::movement::Me {
                id: 1,
                position: [30.0, 40.0, 0.0],
                group: -1,
                action_state: g.state,
            },
            dt: 0.1,
            world: &mut world,
            plan: None,
        };
        let (mut kind, mut held) = (-1i16, 0.0f32);
        let mut sprite = None;
        let (mut rng, mut action) = (Well512::default(), Well512::default());
        let mut gatherer = Gatherer {
            held_kind: &mut kind,
            held: &mut held,
            work_rate: 0.31,
            capacity: 10.0,
            productivity: 1.0,
            sprites: HuntSprites::default(),
            sprite: &mut sprite,
            sprite_rng: &mut rng,
            class: 4,
            master,
            stance: 0,
            world_time: 0,
            farm_stops: false,
            posted: Vec::new(),
            action_rng: &mut action,
            player: 7,
            computer: false,
            stop_productivity: 1.0,
            food_sold: 0.0,
            sold: 0.0,
            anim_stopped: false,
        };
        g.update(walk, None, &mut gatherer, dt)
    };
    let mut g = GatherAction {
        state: WALK,
        target: -1,
        move_to: Some(MoveTo::Moving),
        ..GatherAction::new(task(), &bush(), false, rate()).unwrap()
    };
    assert_eq!(run(120, &mut g, 0.02), Ok(Next::Search));
    assert_eq!((g.state, g.timer, g.move_to), (SEARCH, -10.0, None));
    // state 3 counts the world's dt up to 0, then waits 10 seconds again and asks again
    g.timer = -0.03;
    assert_eq!(run(120, &mut g, 0.02), Ok(Next::Stay));
    assert_eq!(run(120, &mut g, 0.02), Ok(Next::Search));
    assert_eq!(g.timer, -10.0);
    // a farmer waits a second for another farm instead (set_state 0xf3), its unit AI asked
    let mut g = GatherAction {
        state: WORK,
        target: -1,
        ..GatherAction::new(task(), &bush(), false, rate()).unwrap()
    };
    assert_eq!(run(259, &mut g, 0.02), Ok(Next::Search));
    assert_eq!((g.state, g.timer), (FARM_WAIT, -1.0));
}

/// A gatherer's update in `state` with `target`, the world's dt `dt`, a human's unless
/// `computer`: what it answers, with the gather after it.
fn farm_update(
    g: &mut GatherAction,
    target: Option<&mut Victim>,
    dt: f32,
    computer: bool,
) -> Result<Next, GenieError> {
    let mut mover = crate::gathering::hunt::tests::walker();
    let mut world = crate::map::spatial::Spatial::default();
    let walk = Walk {
        mover: Some(&mut mover),
        me: crate::movement::Me {
            id: 1,
            position: [30.0, 40.0, 0.0],
            group: -1,
            action_state: g.state,
        },
        dt: 0.1,
        world: &mut world,
        plan: None,
    };
    let (mut kind, mut held) = (-1i16, 0.0f32);
    let mut sprite = None;
    let (mut rng, mut action) = (Well512::default(), Well512::default());
    let mut gatherer = Gatherer {
        held_kind: &mut kind,
        held: &mut held,
        work_rate: 0.31,
        capacity: 10.0,
        productivity: 1.0,
        sprites: HuntSprites::default(),
        sprite: &mut sprite,
        sprite_rng: &mut rng,
        class: 4,
        master: 259,
        stance: 0,
        world_time: 0,
        farm_stops: false,
        posted: Vec::new(),
        action_rng: &mut action,
        player: 7,
        computer,
        stop_productivity: 1.0,
        food_sold: 0.0,
        sold: 0.0,
        anim_stopped: false,
    };
    g.update(walk, target, &mut gatherer, dt)
}

/// Case 5 (`FUN_141162db0`): a gatherer with a turn time turns a facet a turn time; not yet
/// facing its target, the switch breaks and it stays in state 6 for the next update.
#[test]
fn a_gatherer_turning_over_time_faces_its_target_later() {
    let mut g = GatherAction {
        state: FACE,
        target: 28222,
        ..GatherAction::new(task(), &farm(), false, rate()).unwrap()
    };
    let mut mover = crate::gathering::hunt::tests::walker();
    mover.stats.turn_time = 1.0;
    mover.stats.facets = Some(crate::objects::facing::SpriteFacets {
        count: 16,
        flag_1: true,
    });
    mover.angle = 0.0;
    let before = mover.angle;
    let mut world = crate::map::spatial::Spatial::default();
    let walk = Walk {
        mover: Some(&mut mover),
        me: crate::movement::Me {
            id: 1,
            position: [30.0, 40.0, 0.0],
            group: -1,
            action_state: FACE,
        },
        dt: 0.1,
        world: &mut world,
        plan: None,
    };
    let (mut kind, mut held) = (-1i16, 0.0f32);
    let mut sprite = None;
    let (mut rng, mut action) = (Well512::default(), Well512::default());
    let mut gatherer = Gatherer {
        held_kind: &mut kind,
        held: &mut held,
        work_rate: 0.31,
        capacity: 10.0,
        productivity: 1.0,
        sprites: HuntSprites::default(),
        sprite: &mut sprite,
        sprite_rng: &mut rng,
        class: 4,
        master: 259,
        stance: 0,
        world_time: 5000,
        farm_stops: false,
        posted: Vec::new(),
        action_rng: &mut action,
        player: 7,
        computer: false,
        stop_productivity: 1.0,
        food_sold: 0.0,
        sold: 0.0,
        anim_stopped: false,
    };
    // the bush straight behind the unit (the angle 0 faces +x)
    let mut bush = Victim {
        position: [20.0, 40.0, 0.0],
        ..target(10.0)
    };
    assert_eq!(
        g.update(walk, Some(&mut bush), &mut gatherer, 0.1),
        Ok(Next::Stay)
    );
    assert_eq!(g.state, FACE);
    assert_ne!(mover.angle, before);
}

/// What a gatherer's update borrows besides the action: the walk of
/// [`crate::movement::tests::WalkRig`], a load of 3 of resource 17, its sprite and its
/// streams.
struct Rig {
    walker: crate::movement::tests::WalkRig,
    load: (i16, f32),
    sprite: Option<ActiveSprite>,
    streams: (Well512, Well512),
    anim_stopped: bool,
}

impl Rig {
    fn on_the_path() -> Self {
        Self {
            walker: crate::movement::tests::WalkRig::on_the_path(),
            load: (17, 3.0),
            sprite: None,
            streams: (Well512::default(), Well512::default()),
            anim_stopped: false,
        }
    }

    /// `f` on the rig's walk in action state `state` and its gatherer, a human's villager.
    fn run<R>(&mut self, state: u8, f: impl FnOnce(Walk<'_>, &mut Gatherer<'_>) -> R) -> R {
        let walk = self.walker.walk(state);
        let mut g = Gatherer {
            held_kind: &mut self.load.0,
            held: &mut self.load.1,
            work_rate: 0.31,
            capacity: 10.0,
            productivity: 1.0,
            sprites: HuntSprites::default(),
            sprite: &mut self.sprite,
            sprite_rng: &mut self.streams.0,
            class: 4,
            master: 83,
            stance: 0,
            world_time: 0,
            farm_stops: false,
            posted: Vec::new(),
            action_rng: &mut self.streams.1,
            player: 2,
            computer: false,
            stop_productivity: 1.0,
            food_sold: 0.0,
            sold: 0.0,
            anim_stopped: false,
        };
        let r = f(walk, &mut g);
        self.anim_stopped = g.anim_stopped;
        r
    }
}

/// State 8's drop-off (`0x140a8e1d0` case 7) by its `MoveTo`: the timer's wait (below 0 it
/// counts up, at 0 the drop-off starts again); a waiting `MoveTo` starts again past its
/// delay; a failed one waits a random while, the animation stopped; an arrived one stops the
/// unit, then a site gone starts again, a site that does not take the load idles, one out of
/// reach waits, and one in reach takes the load, as its player's resource kind.
#[test]
fn a_gatherer_s_drop_off_waits_walks_and_drops_its_load_at_the_site() {
    let dropping = |move_to, timer| GatherAction {
        state: DROP,
        carry_to: 10028,
        move_to,
        timer,
        ..GatherAction::new(task(), &bush(), true, rate()).unwrap()
    };
    let site = |takes, gap_squared| Some(crate::gathering::hunt::DropOff { takes, gap_squared });
    let drop = |g: &mut GatherAction, rig: &mut Rig, site| {
        rig.run(DROP, |walk, gatherer| {
            g.update_drop(walk, gatherer, 4, site)
        })
    };
    let mut rig = Rig::on_the_path();
    let mut g = dropping(Some(MoveTo::Arrived), -0.15);
    assert_eq!(drop(&mut g, &mut rig, None), Ok(Dropped::Going));
    assert_eq!(drop(&mut g, &mut rig, None), Ok(Dropped::Again));
    assert_eq!(g.timer, 0.0);
    let mut g = dropping(Some(MoveTo::Waiting), 0.0);
    g.move_wait.delay = 0.15;
    assert_eq!(drop(&mut g, &mut rig, None), Ok(Dropped::Going));
    assert_eq!(g.move_to, Some(MoveTo::Waiting));
    assert_eq!(drop(&mut g, &mut rig, None), Ok(Dropped::Going));
    assert_eq!((g.move_to, g.move_wait.timer), (Some(MoveTo::Start), 0.0));
    let mut g = dropping(Some(MoveTo::Failed), 0.0);
    assert_eq!(drop(&mut g, &mut rig, None), Ok(Dropped::Going));
    let draw = Well512::default().float01() - WAIT_SHIFT;
    assert_eq!(g.timer, draw);
    assert!(rig.anim_stopped);
    let mut g = dropping(Some(MoveTo::Arrived), 0.0);
    assert_eq!(drop(&mut g, &mut rig, None), Ok(Dropped::Again));
    assert_eq!(g.move_to, None);
    let mut g = dropping(Some(MoveTo::Arrived), 0.0);
    assert_eq!(drop(&mut g, &mut rig, site(false, 0.0)), Ok(Dropped::Idle));
    assert_eq!(g.state, IDLE);
    let mut g = dropping(Some(MoveTo::Arrived), 0.0);
    let far = DROP_REACH + 1.0;
    assert_eq!(drop(&mut g, &mut rig, site(true, far)), Ok(Dropped::Going));
    assert!(g.timer < 0.0);
    let mut g = dropping(Some(MoveTo::Arrived), 0.0);
    assert_eq!(
        drop(&mut g, &mut rig, site(true, 0.0)),
        Ok(Dropped::Load(
            crate::gathering::hunt::resource_kind(17, 4),
            3.0
        ))
    );
    assert_eq!(rig.load, (-1, 0.0));
    let mut g = dropping(None, 0.0);
    assert!(drop(&mut g, &mut rig, None).is_err());
}

/// State 8's `MoveTo` to the drop site (`0x140a8e1d0` case 7, `FUN_140a992b0`): it starts
/// toward the site, asking for a path; moving, it walks on while the site stays on its point
/// and starts again once the site left it.
#[test]
fn a_gatherer_walks_to_its_drop_site_and_starts_again_when_it_moved() {
    let dropping = |move_to| GatherAction {
        state: DROP,
        carry_to: 10028,
        move_to: Some(move_to),
        ..GatherAction::new(task(), &bush(), true, rate()).unwrap()
    };
    let drop = |g: &mut GatherAction, rig: &mut Rig| {
        rig.run(DROP, |walk, gatherer| {
            g.update_drop(walk, gatherer, 4, None)
        })
    };
    let mut rig = Rig::on_the_path();
    rig.walker.mover.paths.clear();
    let mut g = dropping(MoveTo::Start);
    assert_eq!(drop(&mut g, &mut rig), Ok(Dropped::Going));
    assert_eq!(g.move_to, Some(MoveTo::Moving));
    assert_eq!(rig.walker.mover.paths.len(), 2);
    let mut rig = Rig::on_the_path();
    let mut g = dropping(MoveTo::Moving);
    assert_eq!(drop(&mut g, &mut rig), Ok(Dropped::Going));
    assert_eq!(g.move_to, Some(MoveTo::Moving));
    rig.walker.mover.move_to_point = Some([35.0, 88.0, 0.0]);
    assert_eq!(drop(&mut g, &mut rig), Ok(Dropped::Going));
    assert_eq!(g.move_to, Some(MoveTo::Start));
}

/// State 4 (case 3) by its `MoveTo` ([`GatherAction::update_walk`]): it starts toward the
/// target, asking for a path; moving, it walks on while the target stays on its point and
/// starts again once it left; a waiting one starts again past its delay; an arrived one stops
/// the unit and faces the target (`set_state(6)`); a failed one near a target of more than 3
/// workers searches (`set_state(3)`), else waits a random while; none is not modelled.
#[test]
fn a_gatherer_walks_to_its_target_and_faces_it_on_arrival() {
    let walking = |move_to| GatherAction {
        state: WALK,
        target: 10028,
        move_to,
        ..GatherAction::new(task(), &bush(), true, rate()).unwrap()
    };
    let house = |workers| Victim {
        id: 10028,
        position: [30.0, 88.0, 0.0],
        radius: [1.0, 1.0],
        workers,
        ..target(10.0)
    };
    let walk = |g: &mut GatherAction, rig: &mut Rig, workers| {
        let mut house = house(workers);
        rig.run(WALK, |walk, gatherer| {
            g.update(walk, Some(&mut house), gatherer, 0.1)
        })
    };
    let mut rig = Rig::on_the_path();
    rig.walker.mover.paths.clear();
    let mut g = walking(Some(MoveTo::Start));
    assert_eq!(walk(&mut g, &mut rig, 0), Ok(Next::Stay));
    assert_eq!((g.state, g.move_to), (WALK, Some(MoveTo::Moving)));
    assert_eq!(rig.walker.mover.paths.len(), 2);
    let mut rig = Rig::on_the_path();
    let mut g = walking(Some(MoveTo::Moving));
    assert_eq!(walk(&mut g, &mut rig, 0), Ok(Next::Stay));
    assert_eq!(g.move_to, Some(MoveTo::Moving));
    rig.walker.mover.move_to_point = Some([35.0, 88.0, 0.0]);
    assert_eq!(walk(&mut g, &mut rig, 0), Ok(Next::Stay));
    assert_eq!(g.move_to, Some(MoveTo::Start));
    let mut g = walking(Some(MoveTo::Waiting));
    g.move_wait.delay = 0.05;
    assert_eq!(walk(&mut g, &mut rig, 0), Ok(Next::Stay));
    assert_eq!(g.move_to, Some(MoveTo::Start));
    let mut g = walking(Some(MoveTo::Arrived));
    assert_eq!(walk(&mut g, &mut rig, 0), Ok(Next::Stay));
    assert_eq!((g.state, g.move_to, g.timer), (FACE, None, 0.0));
    let mut g = walking(Some(MoveTo::Failed));
    assert_eq!(walk(&mut g, &mut rig, 4), Ok(Next::Search));
    assert_eq!((g.state, g.timer), (SEARCH, SEARCH_WAIT));
    let mut rig = Rig::on_the_path();
    let mut g = walking(Some(MoveTo::Failed));
    assert_eq!(walk(&mut g, &mut rig, 3), Ok(Next::Stay));
    assert_eq!(g.timer, Well512::default().float01() - WAIT_SHIFT);
    assert!(rig.anim_stopped);
    let mut g = walking(None);
    assert!(walk(&mut g, &mut rig, 0).is_err());
}

/// Case 5 facing its target at once (no turn time): with something left in the target it
/// works (`set_state(7)`): the gathering starts at the unit's place, a load of another kind
/// gives way to the task's, and the unit shows its working sprite.
#[test]
fn a_gatherer_facing_its_target_works_it() {
    let working = crate::objects::sprite::Graphic {
        id: 7,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 0,
    };
    let mut rig = Rig::on_the_path();
    rig.walker.mover.stats.turn_time = 0.0;
    rig.load = (2, 4.0);
    let mut g = GatherAction {
        state: FACE,
        target: 10028,
        ..GatherAction::new(task(), &bush(), true, rate()).unwrap()
    };
    let mut house = Victim {
        id: 10028,
        position: [30.0, 88.0, 0.0],
        ..target(10.0)
    };
    let next = rig.run(FACE, |walk, gatherer| {
        gatherer.sprites.working = Some(working);
        g.update(walk, Some(&mut house), gatherer, 0.1)
    });
    assert_eq!(next, Ok(Next::Stay));
    assert_eq!(
        (g.state, g.worked, g.gather_at, g.time_at_stop),
        (WORK, true, [28.0, 89.0], 0.0)
    );
    assert_eq!(rig.load, (17, 0.0));
    assert_eq!(rig.sprite.map(|s| s.graphic.id), Some(7));
}

/// A farmer waiting for another farm (state 0xf3), its farm gone.
fn waiting(worked: bool) -> GatherAction {
    GatherAction {
        state: FARM_WAIT,
        timer: -1.0,
        target: -1,
        target_type: i32::from(FARM_CLASS),
        worked,
        gather_at: [112.0, 173.0],
        ..GatherAction::new(task(), &farm(), false, rate()).unwrap()
    }
}

/// State 0xf3 counts the world's dt up to 0; then a farmer that never gathered idles, one
/// that gathered walks back to where it gathered (state 0xf2).
#[test]
fn a_farmer_without_its_farm_waits_a_second_then_walks_back_to_reseed() {
    let mut g = waiting(true);
    assert_eq!(farm_update(&mut g, None, 0.6, false), Ok(Next::Stay));
    assert_eq!((g.state, g.timer), (FARM_WAIT, -1.0 + 0.6));
    assert_eq!(farm_update(&mut g, None, 0.6, false), Ok(Next::Stay));
    assert_eq!(g.state, RESEED_WALK);
    assert_eq!([g.point[0], g.point[1]], [112.0, 173.0]);
    assert_eq!(
        (g.move_to, g.walk_range),
        (Some(MoveTo::Start), task().range)
    );
    // without a gather point the walk goes to the point it holds
    let mut g = waiting(true);
    g.gather_at = [-1.0, -1.0];
    g.point = [5.0, 6.0, 0.0];
    assert_eq!(farm_update(&mut g, None, 1.0, false), Ok(Next::Stay));
    assert_eq!((g.state, g.point), (RESEED_WALK, [5.0, 6.0, 0.0]));
    // a farmer that never gathered idles
    let mut g = waiting(false);
    assert_eq!(farm_update(&mut g, None, 1.0, false), Ok(Next::Stay));
    assert_eq!((g.state, g.target, g.point), (IDLE, -1, [-1.0; 3]));
    // a timer at 0 or more waits on
    let mut g = waiting(true);
    g.timer = 0.0;
    assert_eq!(farm_update(&mut g, None, 1.0, false), Ok(Next::Stay));
    assert_eq!(g.state, FARM_WAIT);
}

/// A farmer that never gathered, at a farm another farmer works too, walks back to its
/// point to search (state 11, the farm's radius none) and lets the farm go.
#[test]
fn a_farmer_at_a_farm_another_works_walks_back_to_search() {
    let mut g = waiting(false);
    g.target = 12557;
    let mut farm = Victim {
        id: 12557,
        amount: 175.0,
        state: 2,
        class: FARM_CLASS,
        master_id: 50,
        radius: [1.5, 1.5],
        workers: 2,
        ..Victim::default()
    };
    assert_eq!(
        farm_update(&mut g, Some(&mut farm), 1.0, false),
        Ok(Next::Stay)
    );
    assert_eq!((g.state, g.target), (WALK_BACK, -1));
    assert_eq!(g.walk_range, task().range);
}

/// State 0xf2's walk: once it arrived without a target, or failed, the caller reseeds; a
/// target standing (state 2) is faced again.
#[test]
fn at_the_end_of_its_walk_back_a_farmer_reseeds() {
    let walking = |move_to| GatherAction {
        state: RESEED_WALK,
        target: -1,
        move_to: Some(move_to),
        walk_to: Some([112.0, 173.0, 0.0]),
        ..GatherAction::new(task(), &farm(), false, rate()).unwrap()
    };
    let mut g = walking(MoveTo::Arrived);
    assert_eq!(
        farm_update(&mut g, None, 0.1, false),
        Ok(Next::Reseed { arrived: true })
    );
    assert_eq!((g.move_to, g.walk_to), (None, None));
    let mut g = walking(MoveTo::Failed);
    assert_eq!(
        farm_update(&mut g, None, 0.1, false),
        Ok(Next::Reseed { arrived: false })
    );
    // the farm still standing: the farmer faces it (set_state 6)
    let mut g = walking(MoveTo::Arrived);
    g.target = 28222;
    let mut bush = target(10.0);
    assert_eq!(
        farm_update(&mut g, Some(&mut bush), 0.1, false),
        Ok(Next::Stay)
    );
    assert_eq!(g.state, FACE);
    // a dying one (state 3) does not hold it: the reseed
    let mut g = walking(MoveTo::Arrived);
    g.target = 28222;
    let mut dying = Victim {
        state: 3,
        ..target(10.0)
    };
    assert_eq!(
        farm_update(&mut g, Some(&mut dying), 0.1, false),
        Ok(Next::Reseed { arrived: true })
    );
}

/// Without a reseed a computer's farmer whose walk arrived is done; any other idles and
/// lets its targets go.
#[test]
fn without_a_reseed_the_farmer_idles_or_a_computer_s_is_done() {
    let after = |arrived: bool, computer: bool| {
        let mut g = GatherAction {
            state: RESEED_WALK,
            target: 5,
            carry_to: 6,
            move_to: Some(MoveTo::Failed),
            ..GatherAction::new(task(), &farm(), false, rate()).unwrap()
        };
        let mut mover = crate::gathering::hunt::tests::walker();
        let (mut sprite, mut rng) = (None, Well512::default());
        let released = g
            .after_reseed(
                arrived,
                computer,
                Some(&mut mover),
                None,
                &mut sprite,
                &mut rng,
            )
            .unwrap();
        (g.state, released, g.target, g.move_to)
    };
    assert_eq!(after(true, true), (DONE, [-1, -1], 5, None));
    assert_eq!(after(true, false), (IDLE, [5, 6], -1, None));
    assert_eq!(after(false, true), (IDLE, [5, 6], -1, None));
}

#[test]
fn a_forager_walks_to_its_bush() {
    let mut mover = crate::gathering::hunt::tests::walker();
    let mut g = GatherAction::new(task(), &bush(), true, rate()).unwrap();
    assert_eq!((g.stops, g.gather_at, g.target_type), (1, [-1.0; 2], 7));
    let mut held = (2, 0.0);
    g.start(
        &bush(),
        7,
        false,
        false,
        &mut held,
        &mut mover,
        &mut Well512::default(),
    )
    .unwrap();
    assert_eq!(
        (g.state, g.move_to, held),
        (WALK, Some(MoveTo::Start), (2, 0.0))
    );
    // a load of another food becomes the task's kind
    let mut g = GatherAction::new(task(), &bush(), true, rate()).unwrap();
    let mut held = (15, 3.0);
    g.start(
        &bush(),
        7,
        false,
        false,
        &mut held,
        &mut mover,
        &mut Well512::default(),
    )
    .unwrap();
    assert_eq!(held, (17, 3.0));
}

/// Slot 0x48 on the work command's target: the 4v4's forager 31156 walking to bush 28222 (state
/// 4) is sent to it again in update 5026 and walks to it anew (`set_state(4)`); facing or
/// gathering it, it keeps on; a computer player's walking one too. A loaded one sent to its
/// player's drop site takes its load there (`set_state(8)`, the site its second target).
#[test]
fn a_gatherer_sent_again_to_its_bush_walks_to_it_anew() {
    let mut mover = crate::gathering::hunt::tests::walker();
    let walking = |state| GatherAction {
        state,
        move_to: Some(MoveTo::Moving),
        worked: true,
        ..GatherAction::new(task(), &bush(), true, rate()).unwrap()
    };
    let mut g = walking(WALK);
    assert_eq!(
        g.retarget(
            &bush(),
            7,
            false,
            0.0,
            (true, rate()),
            &mut mover,
            &mut Well512::default()
        ),
        Ok(true)
    );
    assert_eq!(
        (
            g.state,
            g.move_to,
            g.target,
            g.carry_to,
            g.point,
            g.target_type
        ),
        (WALK, Some(MoveTo::Start), 28222, -1, [23.5, 93.5, 0.0], 7)
    );
    // the gathering is not started any more (+0x80 cleared first)
    assert!(!g.worked);
    for (state, computer) in [(FACE, false), (WORK, false), (WALK, true)] {
        let mut g = walking(state);
        assert_eq!(
            g.retarget(
                &bush(),
                7,
                computer,
                0.0,
                (true, rate()),
                &mut mover,
                &mut Well512::default()
            ),
            Ok(true)
        );
        assert_eq!((g.state, g.move_to), (state, Some(MoveTo::Moving)));
    }
    let mill = Resource {
        id: 500,
        player: 7,
        drop_site: true,
        ..bush()
    };
    let mut g = walking(WALK);
    assert_eq!(
        g.retarget(
            &mill,
            7,
            false,
            3.0,
            (true, rate()),
            &mut mover,
            &mut Well512::default()
        ),
        Ok(true)
    );
    assert_eq!(
        (g.state, g.target, g.carry_to, g.point),
        (DROP, -1, 500, [-1.0; 3])
    );
}

/// The forager's gathering: `(work rate) × work_val1 × dt` a tick into its load, the bush
/// losing as much; the last bit below 0.1 goes with a thousandth of the master's storage; a
/// load that would pass the capacity asks for the drop-off.
#[test]
fn a_forager_gathers_until_its_load_is_full() {
    let g = GatherAction {
        state: WORK,
        ..GatherAction::new(task(), &bush(), true, rate()).unwrap()
    };
    let (mut kind, mut held) = (17i16, 0.0f32);
    let mut sprite = None;
    let (mut rng, mut action) = (Well512::default(), Well512::default());
    let mut gatherer = Gatherer {
        held_kind: &mut kind,
        held: &mut held,
        work_rate: 0.31,
        capacity: 10.0,
        productivity: 1.0,
        sprites: HuntSprites::default(),
        sprite: &mut sprite,
        sprite_rng: &mut rng,
        class: 4,
        master: 83,
        stance: 0,
        world_time: 0,
        farm_stops: true,
        posted: Vec::new(),
        action_rng: &mut action,
        player: 7,
        computer: false,
        stop_productivity: 1.0,
        food_sold: 0.0,
        sold: 0.0,
        anim_stopped: false,
    };
    let mut bush = target(100.0);
    assert_eq!(g.gather(&mut bush, &mut gatherer, 0.5), Next::Stay);
    assert_eq!((*gatherer.held, bush.amount), (0.155, 100.0 - 0.155));
    // 0.2 less the work leaves a last bit below 0.1: it goes with the storage's thousandth
    let mut bush = target(0.2);
    let last = 0.2f32 - 0.155;
    assert_eq!(g.gather(&mut bush, &mut gatherer, 0.5), Next::Drop);
    assert_eq!(bush.amount, 0.0);
    assert_eq!(*gatherer.held, 0.155 + (0.155 + (125.0 * LAST_BIT + last)));
    *gatherer.held = 9.9;
    assert_eq!(g.gather(&mut target(100.0), &mut gatherer, 0.5), Next::Drop);
    assert_eq!(*gatherer.held, 9.9);
}
