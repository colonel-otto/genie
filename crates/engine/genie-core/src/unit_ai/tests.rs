use super::*;

/// A side whose searches find nothing (`Found::NONE`).
fn finds_nothing() -> impl FnMut(&Search<'_>) -> Result<Found, GenieError> {
    |_| Ok(Found::NONE)
}

impl UnitAi {
    const fn is_idle(&self) -> bool {
        self.current_order == -1 && self.current_action == -1 && self.current_target == -1
    }

    /// The full update of an idle unit (no order, action or target), with its move.
    fn update_idle(
        &mut self,
        dt: u32,
        world_time: u32,
        computer: bool,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
    ) -> Result<Option<AiMove>, GenieError> {
        if !self.is_idle() {
            return Err(GenieError::unsupported("unit is not idle"));
        }
        self.update(dt, world_time, computer, at, ai, side)
            .map(|fx| first_move(&fx))
    }
}

fn idle(ai_type: u32) -> UnitAi {
    UnitAi {
        ai_type,
        current_order: -1,
        current_action: -1,
        current_target: -1,
        adjusted_idle_timeout: 3000,
        look_around_timeout: 1000,
        retarget_timer: 900,
        best_this_update: -1,
        ..UnitAi::default()
    }
}

fn rng() -> Well512 {
    let mut r = Well512::default();
    r.set_seed(0);
    r
}

/// A unit side that records the attack commands, the under-attack flag, the stops (as
/// many attack commands as came before each) and the caps on the last human order time (with
/// as many stops before each).
#[derive(Default)]
struct Recorder {
    attacks: Vec<(i32, bool)>,
    under_attack: Option<bool>,
    stops: Vec<usize>,
    caps: Vec<(u32, usize)>,
    /// The drop-site table's building it answers, and the top action's target.
    drop_site: Option<i32>,
    top: Option<i32>,
}

/// The builder's flood: of two foundations two tiles off, the one left of the unit comes before
/// the one above it (the neighbours go left, right, below, above); one at the edge of the reach
/// (`r − 1` tiles) is tested but its tile is not spread from, one beyond is never reached.
#[test]
fn the_builder_s_flood_tests_left_right_below_above() {
    let at = (50, 50);
    let found = flood_search(at, 8, |x, y| match (x, y) {
        (48, 50) => Some(1),
        (50, 52) => Some(2),
        _ => None,
    });
    assert_eq!(found, Some(1));
    let mut tested = Vec::new();
    assert_eq!(
        flood_search(at, 3, |x, y| {
            tested.push((x, y));
            None
        }),
        None
    );
    // r = 3: tiles less than 2 from the start spread: the 3x3 square, then its neighbours
    assert_eq!(tested.len(), 1 + 4 + 4 + 12);
    assert!(tested.contains(&(52, 50)) && !tested.contains(&(53, 50)));
    assert_eq!(
        &tested[..5],
        &[(50, 50), (49, 50), (51, 50), (50, 49), (50, 51)]
    );
}

impl UnitSide for Recorder {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn table_drop_site(&mut self) -> Result<Option<i32>, GenieError> {
        Ok(self.drop_site)
    }
    fn top_target_now(&mut self) -> Result<Option<i32>, GenieError> {
        Ok(self.top)
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }

    fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        self.attacks.push((target, force));
        Ok(Attacked {
            attacks: true,
            posted: Vec::new(),
        })
    }

    fn set_under_attack(&mut self, value: bool) {
        self.under_attack = Some(value);
    }

    fn stop(&mut self) -> Result<(), GenieError> {
        self.stops.push(self.attacks.len());
        Ok(())
    }

    fn cap_human_order(&mut self, cap: u32) -> Result<(), GenieError> {
        self.caps.push((cap, self.stops.len()));
        Ok(())
    }

    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }

    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }

    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }

    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
}

/// A unit that calls for help: whether the world holds the attacker, the searches it ran
/// (classes and radius) and the attackers its walk got.
struct HelpSide {
    holds: bool,
    searches: Vec<(Vec<i32>, f32)>,
    walks: Vec<i32>,
}

impl UnitSide for HelpSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError> {
        self.searches.push((search.classes.to_vec(), search.radius));
        Ok(Found {
            id: -1,
            enemies: true,
            position: None,
            fresh: true,
            players: 0,
            distance: 0,
            retries: [None; 3],
        })
    }
    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Err(GenieError::unsupported("no attack here"))
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
        Ok(self.holds.then_some(UnitSeen {
            id,
            class: 35,
            owner: 0,
            combat_level: 0,
            state: 4,
            target: None,
            gap_squared: 0.0,
            position: [101.5, 158.5, 0.0],
            hp: 0.0,
            master: 0,
            kind: 0,
            main: None,
            allied: false,
        }))
    }
    fn call_for_help(&mut self, attacker: i32) -> Result<(), GenieError> {
        self.walks.push(attacker);
        Ok(())
    }
}

/// The priest's slot 0x1b8 (`FUN_140c1af00`): faith asked unless `any`; never Gaia, sheep,
/// town centres, walls or a master whose hero flag forbids it; the convert task's attribute
/// (`+0x26`) must be above 0 for the object's class.
#[test]
fn a_priest_converts_by_its_faith_its_task_and_the_object() {
    let attributes = {
        let mut a = vec![0.0f32; 100];
        a[67] = 1.0;
        a[28] = 0.0;
        a
    };
    let tasks = [
        crate::masters::Task {
            action_type: 0x68,
            ..crate::masters::Task::default()
        },
        crate::masters::Task {
            action_type: 0x68,
            object_group: 3,
            attribute_type4: 28,
            ..crate::masters::Task::default()
        },
    ];
    let monk = Converter {
        class: 18,
        master: 125,
        faith: 100.0,
        attributes: &attributes,
        tasks: &tasks,
    };
    let knight = Convertee {
        owner: 2,
        class: 12,
        master: 38,
        hero: 0,
    };
    assert!(can_convert(&monk, Some(&knight), false));
    let tired = Converter {
        faith: 99.0,
        ..monk
    };
    assert!(!can_convert(&tired, Some(&knight), false));
    assert!(can_convert(&tired, Some(&knight), true));
    assert!(!can_convert(&monk, None, true));
    let gaia = Convertee { owner: 0, ..knight };
    assert!(!can_convert(&monk, Some(&gaia), true));
    let sheep = Convertee {
        class: 58,
        ..knight
    };
    assert!(!can_convert(&monk, Some(&sheep), true));
    let tc = Convertee {
        class: 3,
        master: 109,
        ..knight
    };
    assert!(!can_convert(&monk, Some(&tc), true));
    let hero = Convertee { hero: 1, ..knight };
    assert!(!can_convert(&monk, Some(&hero), true));
    // a building's task names attribute 28, which the owner holds at 0
    let house = Convertee {
        class: 3,
        master: 70,
        ..knight
    };
    assert!(!can_convert(&monk, Some(&house), true));
}

/// `FUN_140c42080`: an order joins the history first; a ninth recycles the oldest.
#[test]
fn the_order_history_keeps_the_eight_newest_first() {
    let mut ai = UnitAi::default();
    for t in 0..10 {
        ai.remember_order(OrderRecord {
            order: 700,
            action: 600,
            time: t,
            target: 100 + t as i32,
            category: 1,
            ..OrderRecord::default()
        });
    }
    let targets: Vec<i32> = ai.order_history.iter().map(|r| r.target).collect();
    assert_eq!(targets, [109, 108, 107, 106, 105, 104, 103, 102]);
}

/// The full update's start (`0x140c66b10`): an expired retarget entry goes and a live one
/// stays; a target gone or below 1 hit point empties them all.
#[test]
fn the_retarget_entries_expire_and_a_dead_target_empties_them() {
    let seen = |hp: f32| UnitSeen {
        hp,
        ..UnitSeen::default()
    };
    let mut ai = UnitAi {
        retries: vec![(1, 900), (2, 1100), (3, 1000)],
        ..UnitAi::default()
    };
    ai.expire_retries(1000, &mut |_| Ok(Some(seen(5.0))))
        .unwrap();
    assert_eq!(ai.retries, [(2, 1100)]);
    let mut ai = UnitAi {
        retries: vec![(1, 2000), (2, 2000)],
        ..UnitAi::default()
    };
    ai.expire_retries(1000, &mut |id| Ok((id == 1).then(|| seen(5.0))))
        .unwrap();
    assert_eq!(ai.retries, []);
    let mut ai = UnitAi {
        retries: vec![(1, 2000)],
        ..UnitAi::default()
    };
    ai.expire_retries(1000, &mut |_| Ok(Some(seen(0.5))))
        .unwrap();
    assert_eq!(ai.retries, []);
}

/// `AskForHelp` (`FUN_140c42ce0`): an attacker the world does not hold asks nothing; else the
/// unit searches from its place (slot 0x50, mode 1) over the classes its constructor gave
/// it within its search radius (the line of sight plus 1 when farther; plus 7 for the
/// villager and the fishing ship), the search sets the alert mode, and the walk of its lists
/// looks for helpers. 426048706's gate 7558 (a building, p3) in update 4.
#[test]
fn a_call_for_help_searches_what_the_unit_cares_about_then_walks_the_lists() {
    let at = Surroundings {
        position: [124.5, 174.5, 0.0],
        los: 6.0,
        search_radius: 4.0,
        class: 39,
        ..Surroundings::default()
    };
    let mut gone = HelpSide {
        holds: false,
        searches: Vec::new(),
        walks: Vec::new(),
    };
    let mut gate = idle(BUILDING_AI);
    gate.ask_for_help(7955, &at, false, &mut gone).unwrap();
    assert_eq!((gone.searches.len(), gone.walks.len()), (0, 0));
    let mut side = HelpSide {
        holds: true,
        searches: Vec::new(),
        walks: Vec::new(),
    };
    gate.ask_for_help(7955, &at, false, &mut side).unwrap();
    assert_eq!(side.searches, [(BUILDING_CARED.to_vec(), 7.0)]);
    assert_eq!(side.walks, [7955]);
    assert!(gate.alert_mode);
    let mut villager = idle(CIVILIAN);
    let mut side = HelpSide {
        holds: true,
        searches: Vec::new(),
        walks: Vec::new(),
    };
    villager.ask_for_help(7955, &at, false, &mut side).unwrap();
    assert_eq!(side.searches, [(CIVILIAN_CARED.to_vec(), 14.0)]);
    for (ai_type, class, cared) in [
        (TRANSPORT_AI, 20, &TRANSPORT_CARED[..]),
        (SIEGE_TOWER_AI, 13, &SIEGE_TOWER_CARED[..]),
        (TRADE_AI, 2, &TRADE_SHIP_CARED[..]),
        (TRADE_AI, 19, &TRADE_CART_CARED[..]),
    ] {
        let at = Surroundings {
            class,
            ..at.clone()
        };
        let mut side = HelpSide {
            holds: true,
            searches: Vec::new(),
            walks: Vec::new(),
        };
        idle(ai_type)
            .ask_for_help(7955, &at, false, &mut side)
            .unwrap();
        assert_eq!(side.searches, [(cared.to_vec(), 7.0)]);
    }
    let mut side = HelpSide {
        holds: true,
        searches: Vec::new(),
        walks: Vec::new(),
    };
    idle(FISHING_AI)
        .ask_for_help(7955, &at, false, &mut side)
        .unwrap();
    assert_eq!(side.searches, [(FISHING_CARED.to_vec(), 14.0)]);
}

/// A unit sent off a footprint: what [`UnitSide::off_footprint`] answers.
struct OffSide(OffFootprint, Vec<(i32, [f32; 2])>);

impl UnitSide for OffSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Err(GenieError::unsupported("no attack here"))
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn off_footprint(&mut self, site: i32, radius: [f32; 2]) -> Result<OffFootprint, GenieError> {
        self.1.push((site, radius));
        Ok(self.0)
    }
}

/// Notify 0x216 (`FUN_14123bb20`'s, the base handler's case): the radii come back from the
/// hundredths, the unit walks to the spot (action 0x2bf, the old one the last); a human
/// villager whose action has a target queues the foundation's order in front and that work
/// behind, the points the vectors' x and height.
#[test]
fn a_unit_sent_off_a_footprint_walks_to_the_spot_and_comes_back_to_its_work() {
    let site = Named {
        id: 10028,
        master: 70,
        owner: 1,
        vector: [30.0, 2.0],
    };
    let tree = Named {
        id: 600,
        master: 349,
        owner: 0,
        vector: [33.5, 2.0],
    };
    let off = OffFootprint {
        spot: Some([29.5, 89.5]),
        z: 2.0,
        class: VILLAGER_CLASS,
        computer: false,
        player: 1,
        site: Some(site),
        action_target: Some(tree),
    };
    let mut ai = UnitAi {
        current_action: GATHER_ACTION,
        orders: vec![QueuedOrder {
            order: MOVE_ORDER,
            ..QueuedOrder::default()
        }],
        ..UnitAi::default()
    };
    let mut side = OffSide(off, Vec::new());
    let mut fx = Vec::new();
    let n = Notify::own(9175, SENT_OFF, [100, 150, 10028]);
    assert_eq!(ai.sent_off(n, &mut side, &mut fx), Ok(GO_ON));
    assert_eq!(side.1, vec![(10028, [1.0, 1.5])]);
    assert_eq!(
        fx,
        vec![Effect::MoveAside(AiMove {
            point: [29.5, 89.5, 2.0],
            range: 0.0,
            force: true,
            walk: true,
        })]
    );
    assert_eq!(
        (ai.current_action, ai.last_action),
        (MOVE_ASIDE_ACTION, GATHER_ACTION)
    );
    let orders: Vec<_> = ai
        .orders
        .iter()
        .map(|o| (o.order, o.target, o.point, o.priority, o.ready))
        .collect();
    assert_eq!(
        orders,
        vec![
            (FOUNDATION_ORDER, 10028, [30.0, 2.0, 0.0], 100, true),
            (MOVE_ORDER, 0, [0.0; 3], 0, false),
            (ORDER_ON_TARGET, 600, [33.5, 2.0, 0.0], 100, true),
        ]
    );
}

/// 426049848's deer 14054 (Gaia's, unit-AI type 2, no actions) in update 685: a town centre's
/// foundation sent it off its footprint (0x216). Its move off (slot 0x148 forced: the unit's
/// slot 0x340) gives it its `MoveTo` at once, so the full update's later check (slot 0x430)
/// sees it busy and the action value 0x2bf stays.
#[test]
fn a_unit_sent_off_a_footprint_keeps_its_move_aside_through_the_update() {
    let mut u = idle(2);
    u.post(Notify::own(14054, SENT_OFF, [200, 200, 14100]), 2)
        .unwrap();
    let off = OffFootprint {
        spot: Some([164.5, 51.5]),
        z: 0.0,
        class: PREY_CLASS,
        computer: false,
        player: 0,
        site: None,
        action_target: None,
    };
    let mut side = OffSide(off, Vec::new());
    let at = Surroundings {
        position: [164.5, 50.5, 0.0],
        state: 2,
        class: PREY_CLASS,
        ..Surroundings::default()
    };
    let fx = u
        .update(50, 22643, false, &at, &mut rng(), &mut side)
        .unwrap();
    assert_eq!(
        fx,
        vec![Effect::MoveAside(AiMove {
            point: [164.5, 51.5, 0.0],
            range: 0.0,
            force: true,
            walk: false,
        })]
    );
    assert_eq!((u.current_action, u.last_action), (MOVE_ASIDE_ACTION, -1));
}

/// What the unit's slot 0x388 answers ([`UnitSide::wait_solidify`]), and the foundations
/// asked.
struct WaitSide(bool, Vec<i32>);

impl UnitSide for WaitSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Err(GenieError::unsupported("no attack here"))
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn wait_solidify(&mut self, target: i32) -> Result<bool, GenieError> {
        self.1.push(target);
        Ok(self.0)
    }
}

/// The queue a villager sent off a foundation's footprint holds once its move aside ended
/// (0x2bf, which the full update resets): the foundation's order, then its work.
fn sent_off_queue() -> UnitAi {
    let order = |order: i32, target: i32, x: f32| QueuedOrder {
        issuer: 1,
        order,
        priority: 100,
        target,
        target_owner: 1,
        point: [x, 2.0, 0.0],
        range: 0.0,
        shown: [x, 2.0],
        ready: true,
        target_type: 70,
    };
    UnitAi {
        current_action: -1,
        last_action: MOVE_ASIDE_ACTION,
        current_order: -1,
        order_priority: -1,
        current_target: -1,
        orders: vec![
            order(FOUNDATION_ORDER, 10028, 30.0),
            order(ORDER_ON_TARGET, 600, 33.5),
        ],
        ..UnitAi::default()
    }
}

/// Case 0x2be, the build (`FUN_140c5c170`): `BuildObject` forced (slot 0xf0, `FUN_140c47d80`)
/// gives the unit its build command, which the caller runs after the update in the order the
/// AI gave it ([`Effect::BuildObject`]), and at once makes the build the AI's action and the
/// foundation its target at its place with its class; the tail takes the order and its
/// priority (428326873's villager 8766 at update 1804, whose hunt after it must come later).
#[test]
fn the_build_order_builds_and_takes_the_order_at_once() {
    let mut ai = UnitAi {
        current_action: -1,
        current_order: -1,
        current_target: 7,
        target_type: 10,
        orders: vec![QueuedOrder {
            issuer: 1,
            order: BUILD_ORDER,
            priority: -1,
            target: 10028,
            target_owner: 1,
            point: [30.0, 88.0, 0.0],
            range: 2.0,
            shown: [30.0, 88.0],
            ready: true,
            target_type: 3,
        }],
        ..UnitAi::default()
    };
    let at = Surroundings {
        units: vec![UnitSeen {
            id: 10028,
            class: 3,
            position: [30.0, 88.0, 0.0],
            ..UnitSeen::default()
        }],
        ..scout_at(true, false)
    };
    let mut side = WaitSide(false, Vec::new());
    let order = ai.orders[0];
    let mut fx = Vec::new();
    let r = ai
        .process_order(
            &order,
            Some((&at, &mut side, false)),
            None,
            &mut rng(),
            &mut fx,
        )
        .unwrap();
    assert_eq!((r, fx), (1, vec![Effect::BuildObject(10028)]));
    assert_eq!(
        (
            ai.current_action,
            ai.current_target,
            ai.target_type,
            ai.target_point
        ),
        (BUILD_ACTION, 10028, 3, [30.0, 88.0, 0.0])
    );
    assert_eq!((ai.last_target, ai.last_target_type), (7, 10));
    assert_eq!((ai.current_order, ai.order_priority), (BUILD_ORDER, -1));
}

/// Case 0x2de (`FUN_140c5c170`): slot 0x108 forced (`FUN_140c68a60`) runs the unit's slot
/// 0x388 on the foundation; its 1 makes 0x27a the action value with the old one the last,
/// then the case sets 0x27a again (the last takes it) and the tail takes the order and its
/// priority, which ends the queue's turn: the work waits in the queue.
#[test]
fn a_foundation_order_waits_for_the_foundation_and_ends_the_queue_s_turn() {
    let mut ai = sent_off_queue();
    let mut side = WaitSide(true, Vec::new());
    let fx = ai
        .off_turn(&scout_at(true, false), &mut side, false, &mut rng())
        .unwrap();
    assert_eq!(fx, vec![]);
    assert_eq!(side.1, vec![10028]);
    assert_eq!(
        (ai.current_action, ai.last_action),
        (WAIT_SOLIDIFY_ACTION, WAIT_SOLIDIFY_ACTION)
    );
    assert_eq!(
        (ai.current_order, ai.order_priority),
        (FOUNDATION_ORDER, 100)
    );
    let left: Vec<_> = ai.orders.iter().map(|o| (o.order, o.target)).collect();
    assert_eq!(left, vec![(ORDER_ON_TARGET, 600)]);
}

/// Slot 0x388's 0 (no foundation, or no task on it): the case still makes 0x27a the action
/// value, the old one the last, and returns 0; the order goes and the queue runs the next,
/// the work on its old target (priority 100 passes the action value).
#[test]
fn a_foundation_order_without_a_wait_goes_and_the_work_runs() {
    let mut ai = sent_off_queue();
    let mut side = WaitSide(false, Vec::new());
    // the world holds the work's target
    let at = Surroundings {
        units: vec![UnitSeen {
            id: 600,
            ..UnitSeen::default()
        }],
        ..scout_at(true, false)
    };
    let fx = ai.off_turn(&at, &mut side, false, &mut rng()).unwrap();
    assert_eq!(side.1, vec![10028]);
    assert_eq!(
        (ai.current_action, ai.last_action),
        (WAIT_SOLIDIFY_ACTION, -1)
    );
    assert_eq!((ai.current_order, ai.order_priority), (-1, -1));
    assert_eq!(ai.orders, vec![]);
    let [Effect::OrderOnTarget(work)] = fx.as_slice() else {
        panic!("the work on its target: {fx:?}");
    };
    assert_eq!((work.order, work.target), (ORDER_ON_TARGET, 600));
}

/// Slot 0x108 (`FUN_140c68a60`): unforced, a unit with an action value does not ask its
/// unit; forced or without one it does, and only its 1 changes the action values.
#[test]
fn slot_0x108_asks_the_unit_when_forced_or_without_an_action_value() {
    let mut ai = UnitAi {
        current_action: GATHER_ACTION,
        last_action: 7,
        ..UnitAi::default()
    };
    let mut side = WaitSide(true, Vec::new());
    assert_eq!(ai.wait_for_foundation(10028, false, &mut side), Ok(false));
    assert_eq!(side.1, Vec::<i32>::new());
    assert_eq!((ai.current_action, ai.last_action), (GATHER_ACTION, 7));
    let mut refused = WaitSide(false, Vec::new());
    assert_eq!(ai.wait_for_foundation(10028, true, &mut refused), Ok(false));
    assert_eq!(refused.1, vec![10028]);
    assert_eq!((ai.current_action, ai.last_action), (GATHER_ACTION, 7));
    assert_eq!(ai.wait_for_foundation(10028, true, &mut side), Ok(true));
    assert_eq!(
        (ai.current_action, ai.last_action),
        (WAIT_SOLIDIFY_ACTION, GATHER_ACTION)
    );
    let mut idle = UnitAi {
        current_action: -1,
        ..UnitAi::default()
    };
    assert_eq!(idle.wait_for_foundation(10028, false, &mut side), Ok(true));
    assert_eq!(
        (idle.current_action, idle.last_action),
        (WAIT_SOLIDIFY_ACTION, -1)
    );
}

/// What [`UnitSide::closest_resource`] answers, and the searches asked (group, workers cap,
/// drop site, current target).
#[derive(Default)]
struct ResourceSide {
    /// What `ClosestResourceObject` finds.
    resource: Option<Gatherable>,
    /// The resource searches asked: kind, cap, drop site, current target.
    searches: Vec<(u8, i32, i32, i32)>,
    /// What `ClosestObject` finds (none: no search here).
    closest: Option<Found>,
    /// The foundation the builder's search finds and whether the unit reaches it (none: no
    /// search here).
    foundation: Option<(i32, bool)>,
}

impl UnitSide for ResourceSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        self.closest
            .ok_or(GenieError::unsupported("no search here"))
    }
    fn foundation_search(&mut self) -> Result<i32, GenieError> {
        self.foundation
            .map(|(id, _)| id)
            .ok_or(GenieError::unsupported("no search here"))
    }
    fn reaches(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
        Ok(self.foundation.is_some_and(|(_, reaches)| reaches))
    }
    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Err(GenieError::unsupported("no attack here"))
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn closest_resource(
        &mut self,
        search: ResourceSearch,
    ) -> Result<Option<Gatherable>, GenieError> {
        assert_eq!((search.point, search.min_state), (NO_POINT, 2));
        self.searches
            .push((search.kind, search.cap, search.site, search.current));
        Ok(self.resource)
    }
}

/// What `ClosestObject` finds for the searches' tests: `id` at (5, 6), or none.
fn found_at(id: i32) -> Found {
    Found {
        id,
        enemies: false,
        position: (id != -1).then_some([5.0, 6.0, 0.0]),
        fresh: true,
        players: 0,
        distance: -1,
        retries: [None; 3],
    }
}

/// The villager's case 0x202 on its hunt ([`UnitAi::hunter_search`]): the class is the
/// notify's, else the AI's target class, else its last; with none the target goes and the
/// action stops. A class without a gather group (a deer, 9) takes `ClosestObject`: what it
/// finds it hunts and the processing breaks; finding nothing a human player's villager does
/// nothing more. A group's class searches by kind (a boar's 0x21: food, any workers). A
/// computer player's villager finding nothing, a found object without its place and a farm
/// found are not modelled.
#[test]
fn a_hunter_asked_for_another_target_hunts_what_its_search_finds() {
    let hunter = |target_type, last_target_type| UnitAi {
        ai_type: CIVILIAN,
        current_order: ai_kinds::ORDER_GATHER,
        current_action: HUNT_ACTION,
        current_target: 28202,
        target_type,
        last_target_type,
        ..UnitAi::default()
    };
    let at = Surroundings::default();
    let n = |class| Notify::own(31237, REQUIRES_SEARCH, [HUNT_ACTION, class, 31247]);
    let search = |u: &mut UnitAi, side: &mut ResourceSide, class, computer| {
        let mut fx = Vec::new();
        let r = u.hunter_search(n(class), &at, computer, side, &mut fx);
        (r, fx)
    };
    let deer = || ResourceSide {
        closest: Some(found_at(40)),
        ..ResourceSide::default()
    };
    let mut u = hunter(-1, -1);
    assert_eq!(search(&mut u, &mut deer(), 9, false).0, Ok(BREAK));
    assert_eq!((u.current_target, u.target_type), (40, 9));
    let mut u = hunter(9, -1);
    assert_eq!(search(&mut u, &mut deer(), -1, false).0, Ok(BREAK));
    let mut u = hunter(-1, 9);
    assert_eq!(search(&mut u, &mut deer(), -1, false).0, Ok(BREAK));
    let mut u = hunter(-1, -1);
    assert_eq!(search(&mut u, &mut deer(), -1, false).0, Ok(GO_ON));
    assert_eq!((u.current_target, u.current_action), (-1, -1));
    let mut none = ResourceSide {
        closest: Some(found_at(-1)),
        ..ResourceSide::default()
    };
    let mut u = hunter(-1, -1);
    assert_eq!(search(&mut u, &mut none, 9, false), (Ok(GO_ON), vec![]));
    assert!(search(&mut hunter(-1, -1), &mut none, 9, true).0.is_err());
    let mut placeless = ResourceSide {
        closest: Some(Found {
            position: None,
            ..found_at(40)
        }),
        ..ResourceSide::default()
    };
    assert!(
        search(&mut hunter(-1, -1), &mut placeless, 9, false)
            .0
            .is_err()
    );
    assert!(
        search(&mut hunter(-1, -1), &mut deer(), 49, false)
            .0
            .is_err()
    );
    let mut boar = ResourceSide::default();
    assert_eq!(
        search(&mut hunter(-1, -1), &mut boar, 0x21, false).0,
        Ok(GO_ON)
    );
    assert_eq!(boar.searches, vec![(2, -1, 31247, 28202)]);
}

/// The villager's case 0x202 on its gather ([`UnitAi::gatherer_search`]) beyond the bush:
/// the class is the AI's target class, else its last, else the notify's; with none the
/// target goes and the action stops. Stone (8) is searched below 4 workers. A farm (49, no
/// gather group) takes `ClosestObject`, what it finds gathered; finding nothing a human
/// player's villager does nothing more, a computer player's stops. A found object without
/// its place is not modelled.
#[test]
fn a_gatherer_without_a_class_stops_and_one_on_a_farm_searches_for_another() {
    let gatherer = |target_type, last_target_type| UnitAi {
        ai_type: CIVILIAN,
        current_order: ai_kinds::ORDER_GATHER,
        current_action: GATHER_ACTION,
        current_target: 28202,
        target_type,
        last_target_type,
        ..UnitAi::default()
    };
    let at = Surroundings::default();
    let n = |class| Notify::own(31237, REQUIRES_SEARCH, [GATHER_ACTION, class, 31247]);
    let search = |u: &mut UnitAi, side: &mut ResourceSide, class, computer| {
        let mut fx = Vec::new();
        let r = u.gatherer_search(n(class), &at, computer, side, &mut fx);
        (r, fx)
    };
    let mut u = gatherer(-1, -1);
    assert_eq!(
        search(&mut u, &mut ResourceSide::default(), -1, false).0,
        Ok(GO_ON)
    );
    assert_eq!((u.current_target, u.current_action), (-1, -1));
    let mut stone = ResourceSide::default();
    let _ = search(&mut gatherer(-1, 8), &mut stone, -1, false);
    assert_eq!(stone.searches, vec![(3, 4, 31247, 28202)]);
    let mut farm = ResourceSide {
        closest: Some(found_at(12557)),
        ..ResourceSide::default()
    };
    let mut u = gatherer(-1, -1);
    let (r, fx) = search(&mut u, &mut farm, 49, false);
    assert_eq!((r, fx), (Ok(BREAK), vec![Effect::GatherObject(12557)]));
    let mut none = ResourceSide {
        closest: Some(found_at(-1)),
        ..ResourceSide::default()
    };
    let mut u = gatherer(-1, -1);
    assert_eq!(search(&mut u, &mut none, 49, false), (Ok(GO_ON), vec![]));
    assert_eq!(u.current_target, 28202);
    let mut u = gatherer(-1, -1);
    assert_eq!(search(&mut u, &mut none, 49, true).0, Ok(GO_ON));
    assert_eq!(u.current_target, -1);
    let mut placeless = ResourceSide {
        closest: Some(Found {
            position: None,
            ..found_at(40)
        }),
        ..ResourceSide::default()
    };
    assert!(
        search(&mut gatherer(-1, -1), &mut placeless, 49, false)
            .0
            .is_err()
    );
}

/// The villager's cases 0x1fb, 0x1fc and 0x202 by its action ([`UnitAi::villager_search`]):
/// the hunt and the gather search as theirs; the builder takes the foundation its search
/// finds and reaches (the processing breaks), finding none or not reaching it a human
/// player's stops; the shepherd's search, a computer builder without a foundation and the
/// explorer's search are not modelled.
#[test]
fn a_villager_searches_by_its_action() {
    let villager = |current_action| UnitAi {
        ai_type: CIVILIAN,
        current_action,
        current_target: 28202,
        target_type: 9,
        ..UnitAi::default()
    };
    let at = Surroundings {
        units: vec![UnitSeen {
            id: 10028,
            class: 3,
            position: [7.0, 8.0, 0.0],
            ..UnitSeen::default()
        }],
        ..Surroundings::default()
    };
    let n = Notify::own(31237, REQUIRES_SEARCH, [0, -1, -1]);
    let run = |u: &mut UnitAi, side: &mut ResourceSide, computer| {
        let mut fx = Vec::new();
        let r = u.villager_search(n, &at, computer, side, &mut fx);
        (r, fx)
    };
    let mut deer = ResourceSide {
        closest: Some(found_at(40)),
        ..ResourceSide::default()
    };
    assert_eq!(
        run(&mut villager(HUNT_ACTION), &mut deer, false).0,
        Ok(BREAK)
    );
    let mut u = villager(GATHER_ACTION);
    assert_eq!(run(&mut u, &mut deer, false).0, Ok(BREAK));
    let building = |foundation| ResourceSide {
        foundation: Some(foundation),
        ..ResourceSide::default()
    };
    let mut u = villager(BUILD_ACTION);
    let (r, fx) = run(&mut u, &mut building((10028, true)), false);
    assert_eq!((r, fx), (Ok(BREAK), vec![Effect::BuildObject(10028)]));
    assert_eq!((u.current_target, u.target_type), (10028, 3));
    for foundation in [(10028, false), (-1, false)] {
        let mut u = villager(BUILD_ACTION);
        let got = run(&mut u, &mut building(foundation), false);
        assert_eq!(got, (Ok(BREAK), vec![Effect::Stop]), "{foundation:?}");
    }
    let mut u = villager(BUILD_ACTION);
    assert!(run(&mut u, &mut building((-1, false)), true).0.is_err());
    for action in [SHEPHERD_ACTION, EXPLORE_ACTION] {
        let mut u = villager(action);
        assert!(run(&mut u, &mut ResourceSide::default(), false).0.is_err());
    }
}

/// The villager's notify 0x202 on its gather (the 4v4's forager 31237 after update 10822):
/// its target class (7, a bush: gather group 1) names the search from the notify's drop
/// site, any workers; what it finds it gathers, the AI's target and action taken.
#[test]
fn a_gatherer_asked_for_another_target_gathers_what_its_search_finds() {
    let bush = Gatherable {
        id: 28200,
        position: [111.5, 16.5, 0.0],
        class: 7,
    };
    let mut ai = UnitAi {
        ai_type: CIVILIAN,
        current_order: ai_kinds::ORDER_GATHER,
        current_action: GATHER_ACTION,
        current_target: 28202,
        target_type: 7,
        ..UnitAi::default()
    };
    let mut side = ResourceSide {
        resource: Some(bush),
        ..ResourceSide::default()
    };
    let mut fx = Vec::new();
    let at = Surroundings::default();
    let n = Notify::own(31237, REQUIRES_SEARCH, [GATHER_ACTION, 7, 31247]);
    assert_eq!(
        ai.gatherer_search(n, &at, false, &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(side.searches, vec![(1, -1, 31247, 28202)]);
    assert_eq!(fx, vec![Effect::GatherObject(28200)]);
    assert_eq!(
        (ai.current_target, ai.last_target, ai.target_point),
        (28200, 28202, [111.5, 16.5, 0.0])
    );
    // gold is searched below 4 workers; finding nothing a human's villager stays as it is
    let mut ai = UnitAi {
        ai_type: CIVILIAN,
        current_action: GATHER_ACTION,
        target_type: 32,
        ..UnitAi::default()
    };
    let mut side = ResourceSide {
        resource: None,
        ..ResourceSide::default()
    };
    let mut fx = Vec::new();
    let n = Notify::own(31237, REQUIRES_SEARCH, [GATHER_ACTION, 32, 0]);
    assert_eq!(
        ai.gatherer_search(n, &at, false, &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!((side.searches[0].0, side.searches[0].1), (4, 4));
    assert_eq!(fx, Vec::new());
    assert_eq!(ai.current_action, GATHER_ACTION);
}

/// The villager's notify 0x202 on its hunt (`FUN_140c22bf0`, action 0x265) of a class the
/// civilian's slot 0x1b0 names a kind of: `ClosestResourceObject` (slot 0x80) from the
/// notify's drop site, kind 0 below 2 workers, another any; what it finds it hunts
/// (`HuntObject`, slot 0xc8 with 1). Finding nothing a human player's villager stays.
#[test]
fn a_hunter_of_a_kind_searches_by_it_from_its_drop_site() {
    let tree = Gatherable {
        id: 28300,
        position: [50.5, 60.5, 0.0],
        class: 15,
    };
    let mut ai = UnitAi {
        ai_type: CIVILIAN,
        current_action: 0x265,
        current_target: 28301,
        target_type: 15,
        ..UnitAi::default()
    };
    let mut side = ResourceSide {
        resource: Some(tree),
        ..ResourceSide::default()
    };
    let mut fx = Vec::new();
    let at = Surroundings::default();
    let n = Notify::own(31237, REQUIRES_SEARCH, [0x265, 15, 31247]);
    assert_eq!(
        ai.hunter_search(n, &at, false, &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(side.searches, vec![(0, 2, 31247, 28301)]);
    assert_eq!(fx, vec![Effect::HuntObject(28300)]);
    assert_eq!(ai.current_target, 28300);
    let mut ai = UnitAi {
        ai_type: CIVILIAN,
        current_action: 0x265,
        current_target: -1,
        ..UnitAi::default()
    };
    let mut side = ResourceSide {
        resource: None,
        ..ResourceSide::default()
    };
    let mut fx = Vec::new();
    let n = Notify::own(31237, REQUIRES_SEARCH, [0x265, 32, -1]);
    assert_eq!(
        ai.hunter_search(n, &at, false, &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!(side.searches, vec![(4, -1, -1, -1)]);
    assert_eq!(fx, Vec::new());
}

fn hit_boar() -> UnitAi {
    UnitAi {
        home: Some(Home::default()),
        ..idle(4)
    }
}

fn attacker(id: i32, class: i16, gap_squared: f32) -> UnitSeen {
    UnitSeen {
        id,
        class,
        state: 2,
        target: Some(9729),
        gap_squared,
        position: [100.0, 40.0, 0.0],
        ..UnitSeen::default()
    }
}

/// MarvinBlue/426048731's boar 9729 in update 4132: the hunter 10049's arrow makes it attack
/// the hunter, forced (it was only wandering), furious at a villager; a second attacker's hit
/// then adds to its attackers, but its `AttackObject` is not forced and fails: it keeps the
/// first.
#[test]
fn a_boar_attacks_its_first_attacker_and_keeps_it() {
    let mut u = hit_boar();
    let at = Surroundings {
        id: 9729,
        units: vec![attacker(10049, 4, 9.0), attacker(10050, 4, 1.0)],
        ..Surroundings::default()
    };
    let hit = |caller| Notify {
        caller,
        recipient: 9729,
        kind: DAMAGED,
        p: [caller, 72, 75],
    };
    let mut side = Recorder::default();
    let mut fx = Vec::new();
    let r = u.boar_damaged(hit(10049), 0, &at, &mut rng(), &mut side, &mut fx);
    assert_eq!(r, Ok(BREAK));
    assert_eq!(
        (u.attackers.clone(), side.attacks.clone()),
        (vec![10049], vec![(10049, true)])
    );
    assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 10049));
    assert_eq!(
        (side.under_attack, u.home.map(|h| h.furious)),
        (Some(true), Some(true))
    );
    // the nearer second attacker is the closest now, but the attack is not forced
    let _ = u.boar_damaged(hit(10050), 0, &at, &mut rng(), &mut side, &mut fx);
    assert_eq!(u.attackers, vec![10049, 10050]);
    assert_eq!(side.attacks, vec![(10049, true)]);
    assert_eq!(u.current_target, 10049);
}

/// The 4v4's boar 29825 in update 4599: its Attack on the villager 31126 completed (a stop,
/// slot 0x3c0) and the villager 27240's hit makes it attack again, forced. The stop empties
/// the action list before the attack command fills it, so it runs first, not after the AI.
#[test]
fn a_stop_before_an_attack_command_runs_first() {
    let mut u = UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 31126,
        ..hit_boar()
    };
    let at = Surroundings {
        id: 29825,
        units: vec![attacker(27240, 4, 1.0)],
        ..Surroundings::default()
    };
    let mut side = Recorder::default();
    let mut fx = Vec::new();
    let done = Notify::own(29825, ACTION_COMPLETED, [600, 31126, 0]);
    let r = u.handle(done, 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!((r, fx.clone()), (Ok(GO_ON), vec![Effect::Stop]));
    let hit = Notify {
        caller: 27240,
        recipient: 29825,
        kind: DAMAGED,
        p: [27240, 3, 33],
    };
    let _ = u.handle(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!((side.stops, side.attacks), (vec![0], vec![(27240, true)]));
    assert_eq!(fx, vec![]);
    assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 27240));
}

/// `FUN_140c63e20`'s alert: a fresh search's enemies set the alert mode and start the time
/// since the last sighting again; a fresh one without clears the alert mode; a search that
/// read the last answer changes nothing.
#[test]
fn a_fresh_search_s_enemies_set_the_alert_mode() {
    let found = |fresh, enemies| Found {
        enemies,
        fresh,
        ..Found::NONE
    };
    let mut u = UnitAi {
        time_since_enemy: 12.0,
        ..idle(CIVILIAN)
    };
    u.alert(found(true, true), 0.05);
    assert_eq!((u.alert_mode, u.time_since_enemy), (true, 0.0));
    u.time_since_enemy = 3.0;
    u.alert(found(false, false), 0.05);
    assert_eq!((u.alert_mode, u.time_since_enemy), (true, 3.0));
    u.alert(found(true, false), 0.05);
    assert_eq!((u.alert_mode, u.time_since_enemy), (false, 3.0));
}

/// 428506340's Gaia 31602 (update 4609): a cap on the last human order time and a stop wait
/// when an attack command comes; both run before it, in their order. Another effect still
/// stops the slice.
#[test]
fn an_attack_command_runs_the_pending_cap_and_stop_first() {
    let mut u = hit_boar();
    let at = Surroundings {
        id: 31602,
        ..Surroundings::default()
    };
    let mut side = Recorder::default();
    let mut fx = vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP), Effect::Stop];
    assert_eq!(
        u.attack_command(27240, true, &at, &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(
        (side.caps, side.stops, side.attacks, fx),
        (
            vec![(HUMAN_ORDER_CAP, 0)],
            vec![0],
            vec![(27240, true)],
            vec![]
        )
    );
    let mut fx = vec![Effect::Stop, Effect::RestorePatrol];
    assert!(
        u.attack_command(27240, true, &at, &mut Recorder::default(), &mut fx)
            .is_err()
    );
}

/// The 4v4's boar 29823 in update 2917 walks to its villager and meets another of the same
/// master: a boar (class 10 of combat level 1) lets it be; a wolf (combat level 4) with no
/// order attacks it, forced, unless its stance is not aggressive.
#[test]
fn a_unit_in_the_way_is_attacked_by_a_wolf_not_by_a_boar() {
    let at = |combat_level| Surroundings {
        id: 29823,
        class: CLASS_10,
        combat_level,
        units: vec![attacker(29001, 4, 1.0)],
        ..Surroundings::default()
    };
    let n = Notify::own(29823, IN_THE_WAY, [29001, -1, -1]);
    let mut side = Recorder::default();
    let mut fx = Vec::new();
    let mut boar = UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 29002,
        ..hit_boar()
    };
    let r = boar.handle(n, 0, &at(1), false, &mut rng(), &mut side, &mut fx);
    assert_eq!((r, side.attacks.clone()), (Ok(GO_ON), vec![]));
    let mut wolf = UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 29002,
        ..idle(5)
    };
    let r = wolf.handle(n, 0, &at(4), false, &mut rng(), &mut side, &mut fx);
    assert_eq!((r, side.attacks.clone()), (Ok(GO_ON), vec![(29001, true)]));
    assert_eq!(wolf.current_target, 29001);
    let mut defensive = UnitAi {
        stance: 1,
        ..idle(5)
    };
    let mut side = Recorder::default();
    let _ = defensive.handle(n, 0, &at(4), false, &mut rng(), &mut side, &mut fx);
    assert_eq!(side.attacks, []);
}

/// A lion's world: the answers of its two searches (a villager, then any enemy), the object
/// it then reads, and the attack commands it gave.
struct LionSide {
    found: [i32; 2],
    searches: usize,
    seen: UnitSeen,
    attacks: Vec<(i32, bool)>,
}

impl UnitSide for LionSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        let id = self.found[self.searches];
        self.searches += 1;
        Ok(Found {
            id,
            enemies: id != -1,
            position: Some([0.0; 3]),
            fresh: true,
            players: 0,
            distance: -1,
            retries: [None; 3],
        })
    }
    fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        self.attacks.push((target, force));
        Ok(Attacked {
            attacks: true,
            posted: Vec::new(),
        })
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
        Ok((id == self.seen.id).then_some(self.seen))
    }
}

/// The lion's idle check (`FUN_140c1fb70`): a villager it finds within half its line of
/// sight (outside mode 1) it attacks, not forced; finding none, any enemy within a third.
#[test]
fn an_idle_lion_attacks_what_it_finds_within_its_reach() {
    let run = |found: [i32; 2], gap_squared: f32| {
        let mut lion = idle(5);
        let at = Surroundings {
            los: 4.0,
            search_radius: 1.0,
            ..Surroundings::default()
        };
        let mut side = LionSide {
            found,
            searches: 0,
            seen: UnitSeen {
                id: found[0].max(found[1]),
                class: 4,
                owner: 1,
                state: 2,
                gap_squared,
                ..UnitSeen::default()
            },
            attacks: Vec::new(),
        };
        let mut fx = Vec::new();
        let produced = lion.lion_idle(&at, &mut rng(), &mut side, &mut fx).unwrap();
        // FUN_140c41e50 first
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
        (
            produced,
            side.attacks,
            lion.current_action,
            lion.current_target,
        )
    };
    // a villager: the reach is 2 (half of 4); slot 0xb8 then gives 6 whatever it did
    assert_eq!(
        run([700, -1], 3.9),
        (true, vec![(700, false)], ATTACK_ACTION, 700)
    );
    assert_eq!(run([700, -1], 4.1), (false, vec![], -1, -1));
    // any enemy: a third of 4
    assert_eq!(
        run([-1, 701], 1.7),
        (true, vec![(701, false)], ATTACK_ACTION, 701)
    );
    assert_eq!(run([-1, 701], 1.8), (false, vec![], -1, -1));
}

/// A soldier's world: what its `bestUnitToAttack` finds and the attack commands it gave.
struct SoldierSide {
    found: i32,
    attacks: Vec<(i32, bool)>,
    patrol: Option<(PatrolPath, i32)>,
    /// Slot 0xa0 refuses every target.
    refuses: bool,
}

impl UnitSide for SoldierSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        self.patrol.clone()
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        Ok(Found {
            id: self.found,
            enemies: self.found != -1,
            position: Some([10.0, 11.0, 0.0]),
            fresh: true,
            players: 0,
            distance: -1,
            retries: [None; 3],
        })
    }
    fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        self.attacks.push((target, force));
        Ok(Attacked {
            attacks: true,
            posted: Vec::new(),
        })
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Ok((!self.refuses).then_some((58, [10.0, 11.0, 0.0])))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
}

/// A side for the base handler's damage case: the objects it holds, the attack search's
/// answer, the reach, the duels, and what the case asked of it.
#[derive(Default)]
struct DamageSide {
    units: Vec<UnitSeen>,
    found: i32,
    reaches: bool,
    top: Option<i32>,
    duels: Vec<(i32, Duel)>,
    attack_searches: usize,
    attacks: Vec<i32>,
    helps: Vec<i32>,
    under_attack: Vec<bool>,
    patrol: Option<(PatrolPath, i32)>,
    harmless: Vec<i32>,
    defended: Option<i32>,
}

impl UnitSide for DamageSide {
    fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError> {
        let id = if matches!(search.pick, Pick::Attack { .. }) {
            self.attack_searches += 1;
            self.found
        } else {
            -1
        };
        Ok(Found {
            id,
            enemies: id != -1,
            position: None,
            fresh: true,
            players: 0,
            distance: -1,
            retries: [None; 3],
        })
    }
    fn attack(&mut self, target: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        self.attacks.push(target);
        Ok(Attacked {
            attacks: true,
            posted: Vec::new(),
        })
    }
    fn set_under_attack(&mut self, value: bool) {
        self.under_attack.push(value);
    }
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(
        &mut self,
        target: i32,
        _: &Seeker,
    ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Ok(self
            .units
            .iter()
            .find(|u| u.id == target)
            .map(|u| (u.class, u.position)))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        self.patrol.clone()
    }
    fn reaches(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
        Ok(self.reaches)
    }
    fn call_for_help(&mut self, attacker: i32) -> Result<(), GenieError> {
        self.helps.push(attacker);
        Ok(())
    }
    fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
        Ok(self.units.iter().find(|u| u.id == id).copied())
    }
    fn duel(&mut self, other: i32) -> Result<Duel, GenieError> {
        self.duels
            .iter()
            .find(|d| d.0 == other)
            .map(|d| d.1)
            .ok_or(GenieError::unsupported("no duel here"))
    }
    fn top_target_now(&mut self) -> Result<Option<i32>, GenieError> {
        Ok(self.top)
    }
    fn harmless(&mut self, attacker: i32) -> Result<bool, GenieError> {
        Ok(self.harmless.contains(&attacker))
    }
    fn defended_attacker(&mut self, _: i32) -> Result<Option<i32>, GenieError> {
        Ok(self.defended)
    }
    fn can_stand(&mut self, _: [f32; 2]) -> Result<bool, GenieError> {
        Ok(true)
    }
}

/// A human player's villager at (10, 10) on its way to gather (order `0x2c5`, its Gather in
/// state 5, animating) and its unit AI, hit by `foe(30, 6)`'s 4 damage.
fn hit_villager() -> (UnitAi, Surroundings, Notify) {
    let ai = UnitAi {
        current_order: ai_kinds::ORDER_GATHER,
        current_action: GATHER_ACTION,
        current_target: 40,
        order_priority: 3,
        ..idle(CIVILIAN)
    };
    let at = Surroundings {
        id: 20,
        position: [10.0, 10.0, 0.0],
        speed: 1.0,
        class: CLASS_VILLAGER,
        player: 1,
        state: 2,
        map: [100, 100],
        anim_speed: Some(1.0),
        action_state: Some(5),
        units: vec![foe(30, 6)],
        ..Surroundings::default()
    };
    let hit = Notify {
        caller: 30,
        recipient: 20,
        kind: DAMAGED,
        p: [30, 21, 25],
    };
    (ai, at, hit)
}

/// A villager hit by another player's unit it fears ("I have %d attackers, evading them.")
/// calls for help, sends its order to the queue's end, stops its action and runs from the
/// attacker (to its west, the attacker being east of it); the call keeps the attacker.
#[test]
fn a_villager_evades_another_player_s_attacker() {
    let (mut u, at, hit) = hit_villager();
    let mut side = DamageSide {
        units: vec![foe(30, 6)],
        ..DamageSide::default()
    };
    let mut fx = Vec::new();
    let r = u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r, Ok(BREAK));
    assert_eq!((side.helps, side.under_attack), (vec![30], vec![true]));
    assert_eq!(u.attackers, vec![30]);
    let queued: Vec<_> = u
        .orders
        .iter()
        .map(|o| (o.order, o.target, o.issuer))
        .collect();
    assert_eq!(queued, vec![(ai_kinds::ORDER_GATHER, 40, 20)]);
    assert_eq!(fx.first(), Some(&Effect::Stop));
    let Some(Effect::Move(m)) = fx.last() else {
        panic!("no move: {fx:?}");
    };
    assert!(m.point[0] < 10.0 && m.point[1] == 10.0, "{m:?}");
    assert_eq!(side.attacks, Vec::<i32>::new());
    assert_eq!(u.current_action, MOVE_ACTION);
}

/// The villager's cases 0x1fb, 0x1fc and 0x202 off its searching actions: on the move
/// action an invalidated Attack (600) makes the target go and the action stop (records
/// 426339330's villager 34028 in update 5296, running from its attackers); a search (0x202)
/// or the action `0x26a` changes nothing; an invalidated gather or hunt of a class other
/// than 10 would search for another of that class (not modelled).
#[test]
fn a_villager_s_invalidated_action_off_its_searches_stops_it() {
    let moving = || UnitAi {
        current_action: MOVE_ACTION,
        current_target: 30,
        ..idle(CIVILIAN)
    };
    let at = Surroundings {
        id: 20,
        state: 2,
        ..Surroundings::default()
    };
    let n = Notify::own(20, ACTION_INVALIDATED, [ATTACK_ACTION, 0, 514]);
    let mut side = DamageSide::default();
    let mut u = moving();
    let mut fx = Vec::new();
    assert_eq!(
        u.villager_search(n, &at, false, &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!((fx, u.current_target), (vec![Effect::Stop], -1));
    for (kind, action) in [(REQUIRES_SEARCH, MOVE_ACTION), (TARGET_MOVED, 0x26a)] {
        let mut u = UnitAi {
            current_action: action,
            ..moving()
        };
        let mut fx = Vec::new();
        let n = Notify { kind, ..n };
        assert_eq!(
            u.villager_search(n, &at, false, &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!((fx, u.current_target), (Vec::new(), 30));
    }
    let gather = Notify::own(20, ACTION_INVALIDATED, [GATHER_ACTION, 5, -1]);
    assert!(
        moving()
            .villager_search(gather, &at, false, &mut side, &mut Vec::new())
            .is_err()
    );
    let mut fx = Vec::new();
    let tree = Notify::own(20, ACTION_INVALIDATED, [GATHER_ACTION, 10, -1]);
    assert_eq!(
        moving().villager_search(tree, &at, false, &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!(fx, vec![Effect::Stop]);
}

/// The base handler's case `0x1fb` ([`UnitAi::action_invalidated`]): an attack invalidated on
/// the attack order, or on a search (`0x202`), runs processIdle, and with nothing produced a
/// human player's unit forgets its target and stops (a computer player's tests and
/// retargeting off are not modelled); without an order the current action's invalidation
/// forgets the target and stops, with one the order's own action's (action + 100) and any
/// attack's; orders `0x2d4` and `0x2bd` on action `0x262` are not modelled; the rest go on.
#[test]
fn an_invalidated_action_of_the_order_or_an_attack_stops_the_unit() {
    let at = Surroundings {
        id: 20,
        player: 1,
        kind: 70,
        state: 2,
        ..Surroundings::default()
    };
    let unit = |order, action| UnitAi {
        current_order: order,
        current_action: action,
        current_target: 30,
        ..idle(SOLDIER_AI)
    };
    let invalid = |p0, p2| Notify::own(20, ACTION_INVALIDATED, [p0, 0, p2]);
    let run = |u: &mut UnitAi, n, computer| {
        let mut side = DamageSide {
            found: -1,
            ..DamageSide::default()
        };
        let r = u.action_invalidated(n, &at, computer, &mut rng(), &mut side, &mut Vec::new());
        (r, u.current_target)
    };
    let gone = (Ok(GO_ON), -1);
    let kept = (Ok(GO_ON), 30);
    let mut u = unit(ATTACK_ORDER, ATTACK_ACTION);
    assert_eq!(run(&mut u, invalid(ATTACK_ACTION, 0), false), gone);
    assert_eq!(u.current_order, -1);
    let mut u = unit(-1, -1);
    assert_eq!(run(&mut u, invalid(ATTACK_ACTION, 0x202), false), gone);
    let mut u = unit(ATTACK_ORDER, ATTACK_ACTION);
    assert!(run(&mut u, invalid(ATTACK_ACTION, 0), true).0.is_err());
    let mut u = UnitAi {
        retarget_disabled: 1,
        ..unit(ATTACK_ORDER, ATTACK_ACTION)
    };
    assert!(run(&mut u, invalid(ATTACK_ACTION, 0), false).0.is_err());
    let mut u = unit(-1, MOVE_ACTION);
    assert_eq!(run(&mut u, invalid(MOVE_ACTION, 0), false), gone);
    let mut u = unit(-1, MOVE_ACTION);
    assert_eq!(run(&mut u, invalid(GATHER_ACTION, 0), false), kept);
    let mut u = unit(MOVE_ACTION + 100, MOVE_ACTION);
    assert_eq!(run(&mut u, invalid(MOVE_ACTION, 0), false), gone);
    let mut u = unit(0x2c8, MOVE_ACTION);
    assert_eq!(run(&mut u, invalid(MOVE_ACTION, 0), false), kept);
    let mut u = unit(0x2c8, MOVE_ACTION);
    assert_eq!(run(&mut u, invalid(ATTACK_ACTION, 0), false), gone);
    for order in [0x2d4, 0x2bd] {
        let mut u = unit(order, MOVE_ACTION);
        assert!(run(&mut u, invalid(MOVE_ACTION, 0), false).0.is_err());
    }
}

/// The base handler's case `0x1fa` ([`UnitAi::action_completed`]): with an order the state
/// position takes the unit's place; without one the current action's completion, or an
/// attack's, forgets the target and stops; on the attack order an attack's completion runs
/// processIdle, and with nothing produced the target goes and the order ends; with another
/// order its own action's completion stops, order 724 on a move forgets the target and ends,
/// 701 on a move and 712 on a move of action `0x2bf` only drop the action, and an attack's
/// completion stops; the rest go on.
#[test]
fn a_completed_action_of_the_order_or_an_attack_stops_the_unit() {
    let at = Surroundings {
        id: 20,
        position: [12.0, 13.0, 0.0],
        player: 1,
        kind: 70,
        state: 2,
        ..Surroundings::default()
    };
    let unit = |order, action| UnitAi {
        current_order: order,
        current_action: action,
        current_target: 30,
        ..idle(SOLDIER_AI)
    };
    let done = |p0| Notify::own(20, ACTION_COMPLETED, [p0, 0, 0]);
    let run = |u: &mut UnitAi, p0| {
        let mut side = DamageSide {
            found: -1,
            ..DamageSide::default()
        };
        let r = u.action_completed(done(p0), &at, false, &mut rng(), &mut side, &mut Vec::new());
        (r, u.current_target, u.current_action)
    };
    assert_eq!(
        run(&mut unit(-1, MOVE_ACTION), MOVE_ACTION),
        (Ok(GO_ON), -1, -1)
    );
    assert_eq!(
        run(&mut unit(-1, MOVE_ACTION), ATTACK_ACTION),
        (Ok(GO_ON), -1, -1)
    );
    let mut u = unit(-1, MOVE_ACTION);
    assert_eq!(run(&mut u, GATHER_ACTION), (Ok(GO_ON), 30, MOVE_ACTION));
    assert_eq!(u.state_position, [0.0; 2]);
    let mut u = unit(ATTACK_ORDER, ATTACK_ACTION);
    assert_eq!(run(&mut u, ATTACK_ACTION), (Ok(GO_ON), -1, -1));
    assert_eq!((u.current_order, u.state_position), (-1, [12.0, 13.0]));
    let mut u = unit(MOVE_ACTION + 100, MOVE_ACTION);
    assert_eq!(run(&mut u, MOVE_ACTION), (Ok(GO_ON), -1, -1));
    let mut u = unit(0x2d4, MOVE_ACTION);
    assert_eq!(run(&mut u, MOVE_ACTION), (Ok(GO_ON), -1, -1));
    assert_eq!(u.current_order, -1);
    let mut u = unit(0x2bd, MOVE_ACTION);
    assert_eq!(run(&mut u, MOVE_ACTION), (Ok(GO_ON), 30, -1));
    assert_eq!(u.current_order, 0x2bd);
    let mut u = unit(0x2c8, 0x2bf);
    assert_eq!(run(&mut u, MOVE_ACTION), (Ok(GO_ON), 30, -1));
    let mut u = unit(0x2c8, MOVE_ACTION);
    assert_eq!(run(&mut u, MOVE_ACTION), (Ok(GO_ON), 30, MOVE_ACTION));
    let mut u = unit(0x2c8, MOVE_ACTION);
    assert_eq!(run(&mut u, ATTACK_ACTION), (Ok(GO_ON), -1, -1));
}

/// The base handler's kinds (`FUN_140c589c0`, [`UnitAi::base_notify`]): an ownership change
/// stops the unit (its stopAction first), forgets its target and empties its order queue;
/// a moved target without a saved patrol, "I'm ignoring that." and the action values change
/// nothing; readyToAttack's target out of reach gives the attack order up for a unit that
/// cannot move; a unit of a group sent off a footprint takes action value `0x2c0`; an
/// attacker no longer on the unit leaves its set; another kind is not modelled.
#[test]
fn the_base_handler_takes_each_kind_it_models() {
    let at = Surroundings {
        id: 20,
        player: 1,
        kind: 70,
        speed: 1.0,
        state: 2,
        ..Surroundings::default()
    };
    let unit = || UnitAi {
        current_order: ATTACK_ORDER,
        current_action: ATTACK_ACTION,
        current_target: 30,
        attackers: vec![30, 31],
        orders: vec![QueuedOrder::default()],
        ..idle(SOLDIER_AI)
    };
    let run = |u: &mut UnitAi, kind, caller, at: &Surroundings| {
        let mut side = DamageSide {
            found: -1,
            ..DamageSide::default()
        };
        let mut fx = Vec::new();
        let n = Notify::own(caller, kind, [0, 0, 0]);
        let r = u.base_notify(n, 0, at, false, &mut rng(), &mut side, &mut fx);
        (r, fx)
    };
    let mut u = unit();
    let (r, fx) = run(&mut u, OWNER_CHANGED, 20, &at);
    assert_eq!((r, fx.first()), (Ok(GO_ON), Some(&Effect::Stop)));
    assert_eq!((u.current_target, u.orders.len()), (-1, 0));
    for kind in [
        TARGET_MOVED,
        0x1f6,
        0x1ff,
        ACTION_EXECUTED,
        0x203,
        0x25a,
        0x266,
        0x2be,
    ] {
        let mut u = unit();
        assert_eq!(run(&mut u, kind, 20, &at), (Ok(GO_ON), vec![]), "{kind:#x}");
        assert_eq!(u, unit(), "{kind:#x}");
    }
    let mut u = unit();
    assert_eq!(run(&mut u, 0x1fd, 20, &at).0, Ok(GO_ON));
    assert_eq!(u.current_order, ATTACK_ORDER);
    let fixed = Surroundings {
        kind: 20,
        ..at.clone()
    };
    assert_eq!(run(&mut u, 0x1fd, 20, &fixed).0, Ok(GO_ON));
    assert_eq!(u.current_order, -1);
    let mut u = unit();
    assert_eq!(run(&mut u, SENT_OFF_GROUP, 20, &at).0, Ok(GO_ON));
    assert_eq!(u.current_action, GROUP_ASIDE_ACTION);
    let mut u = unit();
    assert_eq!(run(&mut u, NO_LONGER_ON_ME, 30, &at).0, Ok(GO_ON));
    assert_eq!(u.attackers, vec![31]);
    assert_eq!(run(&mut u, NO_LONGER_ON_ME, 77, &at).0, Ok(GO_ON));
    assert_eq!(u.attackers, vec![31]);
    assert!(run(&mut unit(), 0x999, 20, &at).0.is_err());
}

/// A villager dodges a missile's warning by its owner's chance (one AI draw); one it does
/// not dodge is its damage.
#[test]
fn a_villager_dodges_a_missile_s_warning_or_takes_it_as_damage() {
    let r = rng().rand15() as i32 * 100 / 0x7fff;
    let (mut u, mut at, hit) = hit_villager();
    let warning = Notify {
        kind: MISSILE_WARNING,
        ..hit
    };
    at.dodge_chance = u8::try_from(r + 1).unwrap();
    let mut side = DamageSide {
        units: vec![foe(30, 6)],
        ..DamageSide::default()
    };
    let mut ai = rng();
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(warning, 0, &at, false, &mut ai, &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!((ai.draws, fx.len(), u.attackers.len()), (1, 0, 0));
    at.dodge_chance = u8::try_from(r).unwrap();
    assert_eq!(
        u.handle(warning, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(u.attackers, vec![30]);
}

/// A villager's harmless attacker ([`UnitSide::harmless`]) is attacked back, as a Gaia
/// attacker is: without an order straight away (`AttackObject` forced), and it stops after
/// the kill.
#[test]
fn a_villager_attacks_a_harmless_attacker_back() {
    let (mut u, at, hit) = hit_villager();
    u.current_order = -1;
    u.current_action = -1;
    u.current_target = -1;
    let mut side = DamageSide {
        units: vec![foe(30, 6)],
        harmless: vec![30],
        ..DamageSide::default()
    };
    let mut fx = Vec::new();
    let r = u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r, Ok(BREAK));
    assert_eq!(side.attacks, vec![30]);
    assert!(u.stop_after_kill && u.orders.is_empty());
}

/// On the move action (`0x262`) a villager at a speed keeps moving (and returns 3); at a
/// standstill it evades; without its animation speed the slice stops.
#[test]
fn a_moving_villager_keeps_its_way() {
    let (mut u, at, hit) = hit_villager();
    u.current_action = MOVE_ACTION;
    let mut side = DamageSide {
        units: vec![foe(30, 6)],
        ..DamageSide::default()
    };
    let mut fx = Vec::new();
    let walking = u.clone();
    let r = u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r, Ok(BREAK));
    assert!(fx.is_empty() && u.orders.is_empty());
    let nan = Surroundings {
        anim_speed: Some(f32::NAN),
        ..at.clone()
    };
    let mut u = walking.clone();
    let r = u.villager_damaged(hit, 0, &nan, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r, Ok(BREAK));
    assert!(fx.is_empty() && u.orders.is_empty());
    let still = Surroundings {
        anim_speed: Some(0.0),
        ..at.clone()
    };
    let mut u = walking.clone();
    let r = u.villager_damaged(hit, 0, &still, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r, Ok(BREAK));
    assert_eq!(u.orders.len(), 1);
    assert!(matches!(fx.last(), Some(Effect::Move(_))));
    let unknown = Surroundings {
        anim_speed: None,
        ..at
    };
    let mut u = walking;
    assert!(
        u.villager_damaged(
            hit,
            0,
            &unknown,
            false,
            &mut rng(),
            &mut side,
            &mut Vec::new()
        )
        .is_err()
    );
}

/// A friend (player 1, allied) at (11, 10) the soldier guards, `gap_squared` from it.
fn ward(gap_squared: f32) -> UnitSeen {
    UnitSeen {
        id: 20,
        class: 6,
        owner: 1,
        state: 2,
        position: [11.0, 10.0, 0.0],
        gap_squared,
        kind: 70,
        allied: true,
        ..UnitSeen::default()
    }
}

/// A soldier on the defend order of `defend_target` within 2.5.
fn guarding(defend_target: i32) -> UnitAi {
    UnitAi {
        current_order: DEFEND_ORDER,
        current_action: DEFEND_ACTION,
        current_target: defend_target,
        defend_target,
        desired_distance: 2.5,
        target_point: [11.0, 10.0, 0.0],
        ..idle(13)
    }
}

/// processMisc's defend order: a ward the world does not hold, another player's or a dead
/// player's ward ends it (the target goes, the action stops, the order is −1); Gaia's dead
/// ward does not; an unset target point takes the ward's place.
#[test]
fn a_defend_order_ends_with_its_ward_gone_foreign_or_dead() {
    let at = soldier_at();
    let foreign = UnitSeen {
        allied: false,
        ..ward(1.0)
    };
    let dead = UnitSeen {
        state: 3,
        ..ward(1.0)
    };
    for units in [vec![], vec![foreign], vec![dead]] {
        let mut u = guarding(20);
        let mut side = DamageSide {
            units,
            ..DamageSide::default()
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.defend_misc(&at, false, &mut rng(), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(
            (u.defend_target, u.current_target, u.current_order),
            (-1, -1, -1)
        );
        assert!(fx.contains(&Effect::Stop), "{fx:?}");
    }
    let gaia = UnitSeen { owner: 0, ..dead };
    let mut u = guarding(20);
    let mut side = DamageSide {
        units: vec![gaia],
        ..DamageSide::default()
    };
    assert_eq!(
        u.defend_misc(&at, false, &mut rng(), &mut side, &mut Vec::new()),
        Ok(false)
    );
    assert_eq!(u.defend_target, 20);
    // on the attack action only the unset point takes the ward's place
    let mut u = UnitAi {
        current_action: ATTACK_ACTION,
        target_point: [-1.0, -1.0, 3.0],
        ..guarding(20)
    };
    let mut side = DamageSide {
        units: vec![ward(1.0)],
        ..DamageSide::default()
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.defend_misc(&at, false, &mut rng(), &mut side, &mut fx),
        Ok(false)
    );
    assert_eq!((u.target_point, fx), ([11.0, 10.0, 3.0], Vec::new()));
}

/// processMisc's defend order beyond the desired distance plus 1: the soldier walks to its
/// ward (slot 0x138) or runs to the point it defends (slot 0x130, forced); within it, it
/// attacks the ward's nearest attacker (slot 0xb8, forced), or with none its processIdle runs.
#[test]
fn a_defend_order_follows_its_ward_and_attacks_its_attackers() {
    let at = soldier_at();
    let mut u = guarding(20);
    let mut side = DamageSide {
        units: vec![ward(100.0)],
        ..DamageSide::default()
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.defend_misc(&at, false, &mut rng(), &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(
        fx,
        vec![Effect::MoveToObject {
            target: 20,
            state: 2,
            range: 2.5
        }]
    );
    assert_eq!((u.current_action, u.current_target), (MOVE_ACTION, 20));
    // (2.5 + 1)^2 is 12.25: within it the ward's nearest attacker is attacked
    let mut u = guarding(20);
    let mut side = DamageSide {
        units: vec![ward(12.25), foe(40, 6)],
        defended: Some(40),
        ..DamageSide::default()
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.defend_misc(&at, false, &mut rng(), &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(side.attacks, vec![40]);
    // a point to defend, beyond reach: the forced run to it
    let mut u = UnitAi {
        target_point: [20.0, 10.0, 0.0],
        ..guarding(-1)
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.defend_misc(&at, false, &mut rng(), &mut DamageSide::default(), &mut fx),
        Ok(true)
    );
    assert_eq!(
        fx,
        vec![Effect::Move(AiMove {
            point: [20.0, 10.0, 0.0],
            range: 2.5,
            force: true,
            walk: false
        })]
    );
    // within reach of its point, nothing
    let mut u = UnitAi {
        target_point: [12.0, 10.0, 0.0],
        ..guarding(-1)
    };
    assert_eq!(
        u.defend_misc(
            &at,
            false,
            &mut rng(),
            &mut DamageSide::default(),
            &mut Vec::new()
        ),
        Ok(false)
    );
}

/// An enemy (player 2) of `class` the damage case's side holds.
fn foe(id: i32, class: i16) -> UnitSeen {
    UnitSeen {
        id,
        class,
        owner: 2,
        state: 2,
        position: [12.0, 10.0, 0.0],
        hp: 30.0,
        kind: 70,
        ..UnitSeen::default()
    }
}

/// A human player's soldier at (10, 10) that moves.
fn soldier_at() -> Surroundings {
    Surroundings {
        id: 1,
        player: 1,
        kind: 70,
        class: 6,
        speed: 1.0,
        los: 4.0,
        weapon_range: 0.0,
        action_state: Some(0),
        position: [10.0, 10.0, 0.0],
        ..Surroundings::default()
    }
}

fn damage(caller: i32, hp: i32) -> Notify {
    Notify {
        caller,
        recipient: 1,
        kind: DAMAGED,
        p: [0, hp, 0],
    }
}

/// `FUN_140c589c0`'s case 500 up to its attack search: a known attacker of a unit whose top
/// action has a target changes nothing but the retarget entries; the dead give up; an allied
/// attacker is let be once it joined the set; a human player's unit on an order other than
/// a patrol ignores it.
#[test]
fn the_damage_case_lets_known_dead_allied_and_ordered_ones_be() {
    let at = soldier_at();
    let mut fx = Vec::new();
    let mut u = UnitAi {
        attackers: vec![7],
        retries: vec![(3, 900)],
        ..idle(13)
    };
    let mut side = DamageSide {
        units: vec![foe(7, 6)],
        top: Some(9),
        found: 7,
        ..DamageSide::default()
    };
    let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r.unwrap(), GO_ON);
    assert_eq!(u.retries, vec![]);
    assert_eq!(side.under_attack, Vec::<bool>::new());
    assert_eq!(side.attack_searches, 0);

    let mut u = idle(13);
    let mut side = DamageSide {
        units: vec![foe(7, 6)],
        ..DamageSide::default()
    };
    let r = u.damaged(damage(7, 0), 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r.unwrap(), DEAD);
    assert_eq!(
        (u.attackers.clone(), side.under_attack),
        (vec![7], vec![true])
    );

    let mut u = idle(13);
    let mut side = DamageSide {
        units: vec![UnitSeen {
            allied: true,
            ..foe(7, 6)
        }],
        found: 7,
        ..DamageSide::default()
    };
    let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r.unwrap(), GO_ON);
    assert_eq!(u.attackers, vec![7]);
    assert_eq!((side.attack_searches, side.helps.len()), (0, 0));

    let mut u = UnitAi {
        current_order: ATTACK_ORDER,
        ..idle(13)
    };
    let mut side = DamageSide {
        units: vec![foe(7, 6)],
        found: 7,
        ..DamageSide::default()
    };
    let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r.unwrap(), GO_ON);
    // the new attacker made it ask for help first
    assert_eq!((side.attack_searches, side.helps.clone()), (0, vec![7]));
}

/// The unit AIs' class lists by factory type (their constructors' `+0x190`): the animals',
/// the boar's and ranged units' empty ones, the priest's 30, the trade AI's by class, a
/// computer player's soldier's with 10.
#[test]
fn each_unit_ai_type_searches_its_constructor_s_classes() {
    let list = |t: u32, class: i16, computer: bool| cared_classes(t, class, computer).unwrap();
    assert_eq!(list(2, 9, false), &HUNTED_CARED);
    assert_eq!(list(3, 10, false), &[58, 4]);
    let none: &[i32] = &[];
    assert_eq!(list(BOAR_AI, 10, false), none);
    assert_eq!((list(6, 0, false), list(7, 0, false)), (none, none));
    assert_eq!(list(5, 10, false), &LION_CARED);
    assert_eq!(list(PRIEST_AI, 18, false).len(), 30);
    assert_eq!(list(TRADE_AI, TRADE_BOAT_CLASS, false), &TRADE_SHIP_CARED);
    assert_eq!(list(TRADE_AI, 19, false), &TRADE_CART_CARED);
    assert_eq!(list(SOLDIER_AI, 6, true).last(), Some(&10));
    assert_eq!(list(SOLDIER_AI, 6, false), &SOLDIER_CARED);
    assert_eq!(list(LIVESTOCK_AI, 58, false), none);
    assert!(cared_classes(1, 58, false).is_err());
}

/// Slot 0xa8 by factory type: the civilian and the building never attack a neutral object,
/// the base's types always; the predator and the lion spare their five classes, the priest
/// its mask below 50 (61 passes), the tower the soldier's mask; attack priority 3 keeps only
/// its classes.
#[test]
fn each_unit_ai_type_tests_a_neutral_object_by_its_module() {
    for t in [CIVILIAN, BUILDING_AI] {
        assert!(!type_attacks_neutral(t, 0, 6));
    }
    for t in [
        2,
        BOAR_AI,
        6,
        7,
        TRADE_AI,
        TRANSPORT_AI,
        FISHING_AI,
        ARTIFACT_AI,
    ] {
        assert!(type_attacks_neutral(t, 0, 4), "type {t}");
        assert!(type_attacks_neutral(t, 3, 6), "type {t}");
    }
    assert!(type_attacks_neutral(SIEGE_TOWER_AI, 0, 3));
    assert!(type_attacks_neutral(LIVESTOCK_AI, 0, 3));
    for t in [PREDATOR_AI, LION_AI] {
        for class in [13, 22, 52, 54, 55] {
            assert!(!type_attacks_neutral(t, 0, class), "type {t} class {class}");
        }
        assert!(type_attacks_neutral(t, 0, 4));
        assert!(!type_attacks_neutral(t, 3, 6));
    }
    for class in [2, 3, 4, 19, 20, 21, 27, 39, 49] {
        assert!(!type_attacks_neutral(PRIEST_AI, 0, class), "class {class}");
        assert!(!type_attacks_neutral(TOWER_AI, 0, class), "class {class}");
    }
    assert!(type_attacks_neutral(PRIEST_AI, 0, 61));
    assert!(!type_attacks_neutral(TOWER_AI, 0, 61));
    assert!(type_attacks_neutral(PRIEST_AI, 0, 6));
    assert!(!type_attacks_neutral(PRIEST_AI, 3, 6));
    assert_eq!(
        type_attacks_neutral(SIEGE_AI, 0, 19),
        siege_attacks_neutral(0, 19)
    );
}

/// An idle human player's soldier without a target: `bestUnitToAttack` names the attacker,
/// which it reaches and attacks back (3); a second attacker in the same update takes the
/// first search's answer and is ignored. Unreached, it runs (its stance keeps it: 3);
/// nothing found and too slow to run, it lets it be.
#[test]
fn an_idle_soldier_attacks_back_what_best_unit_to_attack_names() {
    let at = soldier_at();
    let mut fx = Vec::new();
    let mut u = idle(13);
    let mut side = DamageSide {
        units: vec![foe(7, 6), foe(8, 6)],
        found: 7,
        reaches: true,
        ..DamageSide::default()
    };
    let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r.unwrap(), BREAK);
    assert_eq!(u.best_this_update, 7);
    assert_eq!(side.attacks, vec![7]);
    assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 7));
    let r = u.damaged(damage(8, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r.unwrap(), GO_ON);
    assert_eq!(side.attack_searches, 1);
    assert_eq!(side.helps, vec![7, 8]);
    assert_eq!(u.attackers, vec![7, 8]);

    let mut u = UnitAi {
        stance: 3,
        ..idle(13)
    };
    let mut side = DamageSide {
        units: vec![foe(7, 6)],
        found: 7,
        ..DamageSide::default()
    };
    let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r.unwrap(), BREAK);
    assert_eq!(side.attacks, Vec::<i32>::new());

    let still = Surroundings {
        speed: 0.0,
        ..soldier_at()
    };
    let mut u = idle(13);
    let mut side = DamageSide {
        units: vec![foe(7, 6)],
        found: -1,
        ..DamageSide::default()
    };
    let r = u.damaged(
        damage(7, 20),
        0,
        &still,
        false,
        &mut rng(),
        &mut side,
        &mut fx,
    );
    assert_eq!(r.unwrap(), GO_ON);
    assert_eq!(u.best_this_update, -1);
}

/// A human player's soldier with a live target attacks back only from a building, wall or
/// tower target; on a patrol its patrol is saved and, the attack taken, the order ends with
/// its timers (an adjusted idle timeout of 750 redrawn, the secondary timer capped).
#[test]
fn a_soldier_with_a_target_attacks_back_from_buildings_only() {
    let at = soldier_at();
    let run = |class: i16, order: i32| {
        let mut fx = Vec::new();
        let mut u = UnitAi {
            current_order: order,
            current_action: ATTACK_ACTION,
            current_target: 9,
            idle_timeout: 3000,
            adjusted_idle_timeout: 750,
            secondary_timer: 900,
            ..idle(13)
        };
        let mut side = DamageSide {
            units: vec![foe(7, 6), foe(9, class)],
            found: 7,
            reaches: true,
            patrol: Some((PatrolPath::default(), 2)),
            ..DamageSide::default()
        };
        let r = u
            .damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx)
            .unwrap();
        (r, u, side.attacks)
    };
    let (r, _, attacks) = run(6, -1);
    assert_eq!((r, attacks), (GO_ON, vec![]));
    for class in [3, 27, 52] {
        let (r, u, attacks) = run(class, -1);
        assert_eq!((r, attacks, u.current_target), (BREAK, vec![7], 7));
    }
    let (r, u, attacks) = run(3, PATROL);
    assert_eq!((r, attacks), (BREAK, vec![7]));
    assert_eq!(u.current_order, -1);
    assert_eq!(u.patrol_waypoint, 2);
    assert!(u.patrol_path.is_some());
    assert_eq!(u.secondary_timer, 750);
    assert_ne!(u.adjusted_idle_timeout, 750);
}

/// The computer player's end of the damage case: a wonder target, a tower against a tower,
/// a missile, and a tower under `disableTowerPriority` (but for a siege weapon) are let be;
/// a harmless target makes it ask for help and attack back, then the kill times decide again
/// (the attacker's sooner, or its conversion, but never against a converting target).
#[test]
fn a_computer_s_unit_weighs_its_target_against_its_attacker() {
    let duel = |converts: bool, attacks: f32, harmed: f32, hp: f32| Duel {
        converts,
        attacks,
        harms: 2.0,
        harmed,
        hp,
    };
    let run = |at: &Surroundings, attacker: UnitSeen, target: UnitSeen, duels| {
        let mut fx = Vec::new();
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: target.id,
            ..idle(13)
        };
        let mut side = DamageSide {
            units: vec![attacker, target],
            duels,
            ..DamageSide::default()
        };
        let r = u
            .computer_weighs(&attacker, &target, at, &mut rng(), &mut side, &mut fx)
            .unwrap();
        (r, side.attacks, side.helps)
    };
    let at = soldier_at();
    let fair = vec![
        (9, duel(false, 5.0, 10.0, 100.0)),
        (7, duel(false, 5.0, 10.0, 50.0)),
    ];
    let wonder = UnitSeen {
        master: crate::turn::WONDER,
        ..foe(9, 3)
    };
    assert_eq!(run(&at, foe(7, 6), wonder, fair.clone()).0, GO_ON);
    let tower = UnitSeen {
        class: 52,
        ..foe(9, 52)
    };
    assert_eq!(
        run(&at, foe(7, 52), tower, fair.clone()),
        (GO_ON, vec![], vec![])
    );
    let missile = UnitSeen {
        kind: 60,
        ..foe(7, 6)
    };
    assert_eq!(run(&at, missile, foe(9, 6), fair.clone()).1, vec![]);
    let shy = Surroundings {
        disable_tower_priority: true,
        ..soldier_at()
    };
    assert_eq!(run(&shy, foe(7, 52), foe(9, 6), fair.clone()).1, vec![]);
    let siege = Surroundings { class: 13, ..shy };
    // the attacker dies in 5 hits, the target in 10
    assert_eq!(run(&siege, foe(7, 52), foe(9, 6), fair.clone()).1, vec![7]);
    assert_eq!(run(&at, foe(7, 6), foe(9, 6), fair).1, vec![7]);
    let slow = vec![
        (9, duel(false, 5.0, 10.0, 100.0)),
        (7, duel(false, 5.0, 10.0, 200.0)),
    ];
    assert_eq!(run(&at, foe(7, 6), foe(9, 6), slow).1, vec![]);
    let converting = vec![
        (9, duel(false, 5.0, 10.0, 100.0)),
        (7, duel(true, 5.0, 10.0, 200.0)),
    ];
    assert_eq!(run(&at, foe(7, 6), foe(9, 6), converting).1, vec![7]);
    let harmless = vec![
        (9, duel(false, 0.0, 10.0, 100.0)),
        (7, duel(false, 5.0, 0.0, 50.0)),
    ];
    // the attack back, then the kill times: the attacker never dies (25000) against 10
    assert_eq!(
        run(&at, foe(7, 6), foe(9, 6), harmless),
        (GO_ON, vec![7], vec![7])
    );
    let priest = vec![
        (9, duel(true, 0.0, 10.0, 100.0)),
        (7, duel(true, 5.0, 10.0, 1.0)),
    ];
    assert_eq!(run(&at, foe(7, 6), foe(9, 6), priest).1, vec![]);
}

/// A scout (master 448) in stand ground whose sheep died (census v4's blocker): its
/// processIdle with 0 (`FUN_140c20db0`) attacks the next sheep `bestUnitToAttack` finds,
/// forced, and its order goes; finding none, it moves to the dead sheep's point.
#[test]
fn a_scout_whose_sheep_died_attacks_the_next_or_walks_to_its_point() {
    let run = |found: i32| {
        let mut scout = UnitAi {
            current_order: ATTACK_ORDER,
            current_action: ATTACK_ACTION,
            current_target: 25954,
            target_type: 58,
            target_point: [5.0, 6.0, 0.0],
            stance: 2,
            ..idle(13)
        };
        let at = Surroundings {
            player: 1,
            kind: 70,
            master: 448,
            class: 47,
            weapon_range: 0.0,
            action_state: Some(0),
            ..Surroundings::default()
        };
        let mut side = SoldierSide {
            found,
            attacks: Vec::new(),
            patrol: None,
            refuses: false,
        };
        let mut fx = Vec::new();
        scout
            .next_after_kill(&at, false, &mut rng(), &mut side, &mut fx)
            .unwrap();
        (scout, side.attacks, fx)
    };
    let (scout, attacks, _) = run(26000);
    assert_eq!(attacks, vec![(26000, true)]);
    assert_eq!(
        (
            scout.current_action,
            scout.current_target,
            scout.current_order
        ),
        (ATTACK_ACTION, 26000, -1)
    );
    assert_eq!((scout.last_order, scout.order_priority), (-1, -1));
    let (scout, attacks, fx) = run(-1);
    assert_eq!(attacks, vec![]);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Move(m) if m.point == [5.0, 6.0, 0.0]))
    );
    assert_eq!((scout.current_order, scout.last_order), (-1, ATTACK_ORDER));
}

/// The target block's attack whose target died on a patrol order: the order ends with its
/// timers reset (an adjusted idle timeout of 750 redrawn, the secondary timer capped).
#[test]
fn a_patrol_s_kill_ends_the_patrol_order_with_its_timers() {
    let mut scout = UnitAi {
        current_order: PATROL,
        current_action: ATTACK_ACTION,
        current_target: 25954,
        target_point: [5.0, 6.0, 0.0],
        idle_timeout: 3000,
        adjusted_idle_timeout: 750,
        secondary_timer: 900,
        ..idle(13)
    };
    let at = Surroundings {
        player: 1,
        kind: 70,
        action_state: Some(0),
        ..Surroundings::default()
    };
    let mut side = SoldierSide {
        found: -1,
        attacks: Vec::new(),
        patrol: None,
        refuses: false,
    };
    scout
        .next_after_kill(&at, false, &mut rng(), &mut side, &mut Vec::new())
        .unwrap();
    assert_eq!(
        (scout.current_order, scout.last_order, scout.secondary_timer),
        (-1, PATROL, 750)
    );
    assert_ne!(scout.adjusted_idle_timeout, 750);
}

/// processMisc's retarget check past its timer (`0x140c567c0`): a scout on the attack order
/// against an enemy's sheep looks no further in stand ground, nor while it swings (its Attack
/// in state 6 or 7); otherwise the search for a better target follows.
#[test]
fn a_retarget_check_stops_in_stand_ground_and_while_attacking() {
    let scout = |stance: u8| UnitAi {
        current_order: ATTACK_ORDER,
        current_action: ATTACK_ACTION,
        current_target: 25954,
        stance,
        ..idle(13)
    };
    let sheep = Some((1, 58));
    assert_eq!(
        scout(2).retarget_outcome(sheep, 0.0, false, Some(4), Some(true)),
        Ok(None)
    );
    assert_eq!(
        scout(0).retarget_outcome(sheep, 0.0, false, Some(6), Some(true)),
        Ok(None)
    );
    assert_eq!(
        scout(0).retarget_outcome(sheep, 0.0, false, Some(4), Some(true)),
        Ok(Some(RetargetSearch {
            building: false,
            work_state: false
        }))
    );
    // a building target is searched past even while attacking
    assert_eq!(
        scout(0).retarget_outcome(Some((1, 3)), 0.0, true, Some(6), Some(true)),
        Ok(Some(RetargetSearch {
            building: true,
            work_state: true
        }))
    );
    // without an Attack in the action list (slot 0x538 with 9) there is no search: a king
    // entering a castle, its target a building (records 426406723's type-2 unit AIs)
    let entering = UnitAi {
        current_order: crate::garrison::ENTER_ORDER,
        current_action: crate::garrison::ENTER_ACTION,
        current_target: 3301,
        stance: 0,
        ..idle(2)
    };
    assert_eq!(
        entering.retarget_outcome(Some((8, 3)), 0.0, false, Some(4), Some(false)),
        Ok(None)
    );
    // an action list the slice does not hold stops at the state's test
    assert!(
        entering
            .retarget_outcome(Some((8, 3)), 0.0, false, None, None)
            .is_err()
    );
}

#[test]
fn a_lion_attacks_by_its_priority_or_its_classes() {
    assert!(lion_may_attack(1, 3));
    assert!(!lion_may_attack(3, 4));
    assert!(lion_may_attack(3, 3));
    assert!(lion_may_attack(0, 4));
    assert!(!lion_may_attack(0, 13));
}

/// MarvinBlue/426048731's villager 10041 in update 4839, hunting the boar 9731 (its Hunt in
/// state 12) that hits it: the boar is its target, so it calls nobody for help; it is under
/// attack, and the hunt order with a class-10 target and a Gaia attacker ignores the damage.
#[test]
fn a_hunter_hit_by_its_boar_ignores_the_damage() {
    let mut u = UnitAi {
        current_order: ai_kinds::ORDER_HUNT,
        current_action: 613,
        current_target: 9731,
        ..idle(8)
    };
    let boar = UnitSeen {
        owner: 0,
        ..attacker(9731, CLASS_10, 0.5)
    };
    let at = Surroundings {
        id: 10041,
        action_state: Some(12),
        target: Some(Seen {
            class: CLASS_10,
            ..Seen::default()
        }),
        units: vec![boar],
        ..Surroundings::default()
    };
    let hit = Notify {
        caller: 9731,
        recipient: 10041,
        kind: DAMAGED,
        p: [9731, 34, 40],
    };
    let mut side = Recorder::default();
    let mut fx = Vec::new();
    let r = u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r, Ok(GO_ON));
    assert_eq!(
        (u.attackers.clone(), side.under_attack),
        (vec![9731], Some(true))
    );
    assert_eq!(
        (u.current_order, u.current_target),
        (ai_kinds::ORDER_HUNT, 9731)
    );
    // another player's attacker that is not its target would make it call for help
    let enemy = UnitSeen {
        owner: 1,
        ..attacker(9800, 4, 1.0)
    };
    let at = Surroundings {
        units: vec![enemy],
        ..at
    };
    let hit = Notify {
        caller: 9800,
        ..hit
    };
    assert!(
        u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx)
            .is_err()
    );
}

#[test]
fn closest_valid_attacker_skips_classes_and_flips_a_coin_on_a_tie() {
    let mut u = hit_boar();
    u.attackers = vec![1, 2, 3];
    let at = Surroundings {
        units: vec![
            attacker(1, 13, 0.5),
            attacker(2, 4, 4.0),
            attacker(3, 4, 4.03),
        ],
        ..Surroundings::default()
    };
    // 1 is of class 13; 3 lies within 0.0625 of 2: the coin decides
    let mut r = rng();
    let mut coin = rng();
    let picked = u.closest_valid_attacker(&at, &mut r);
    let expected = if coin.rand15() & 1 == 0 { 2 } else { 3 };
    assert_eq!(picked, expected);
}

#[test]
fn an_attacker_that_no_longer_targets_the_unit_leaves_one_an_update() {
    let mut u = hit_boar();
    u.attackers = vec![10049, 10050];
    let mut walked_off = attacker(10049, 4, 1.0);
    walked_off.target = Some(-1);
    let mut dead = attacker(10050, 4, 1.0);
    dead.state = 3;
    let at = Surroundings {
        id: 9729,
        under_attack: true,
        units: vec![walked_off, dead],
        ..Surroundings::default()
    };
    let mut side = Recorder::default();
    u.keep_attackers(&at, &mut side);
    assert_eq!(u.attackers, vec![10050]);
    u.keep_attackers(&at, &mut side);
    assert_eq!(u.attackers, Vec::<i32>::new());
    assert_eq!(side.under_attack, None);
    u.keep_attackers(&at, &mut side);
    assert_eq!(side.under_attack, Some(false));
}

/// MarvinBlue/426048731's boar 9729 in updates 4157 to 4159: the hunter 10049 moves off, its
/// Hunt's teardown lets the boar go (notify 699 with the hunter's class); the boar's AI takes
/// it, as 10049 is one of its attackers, and its base handler drops the attacker. A 699 from
/// a unit that is not attacking it is ignored.
#[test]
fn a_released_attacker_leaves_the_set_through_notify_699() {
    let mut u = hit_boar();
    u.attackers = vec![10049];
    let released = |caller| Notify {
        caller,
        recipient: 9729,
        kind: NO_LONGER_ON_ME,
        p: [4, 0, 0],
    };
    u.post(released(10050), 2).unwrap();
    assert_eq!(u.notifies, Vec::new());
    u.post(released(10049), 2).unwrap();
    assert_eq!(u.notifies, vec![released(10049)]);
    let r = u.base_notify(
        released(10049),
        0,
        &Surroundings::default(),
        false,
        &mut rng(),
        &mut Recorder::default(),
        &mut Vec::new(),
    );
    assert_eq!((r, u.attackers.clone()), (Ok(GO_ON), Vec::new()));
}

/// A builder on its build order (702, action 602): MarvinBlue/426048731's 9175 in update 681,
/// whose look-around timer runs out and is redrawn from the AI stream.
#[test]
fn a_builder_s_idle_timer_stays_0_and_its_look_around_is_redrawn() {
    let mut u = UnitAi {
        current_order: BUILD_ORDER,
        current_action: BUILD_ACTION,
        current_target: 10028,
        idle_timer: 120,
        secondary_timer: 10,
        look_around_timer: 990,
        ..idle(8)
    };
    let mut r = rng();
    let at = Surroundings {
        position: [29.5, 89.5, 0.0],
        busy: true,
        player: 2,
        class: 4,
        ..Surroundings::default()
    };
    let _ = u.update(50, 0, false, &at, &mut r, &mut no_search).unwrap();
    assert_eq!(
        (u.idle_timer, u.secondary_timer, u.look_around_timer),
        (0, 0, 0)
    );
    assert_eq!(u.look_around_timeout, 4500 + 7241 % 1000);
    assert_eq!(r.draws, 1);
    // the state position follows a unit with an order
    assert_eq!(u.state_position, [29.5, 89.5]);
    // on the attack order, its target gone (the target block forgets it), the civilian's
    // processMisc stops the action and ends the order (slot 0xb0 with 0)
    u.current_order = 700;
    let fx = u.update(50, 0, false, &at, &mut r, &mut no_search).unwrap();
    assert_eq!(fx, [Effect::Stop]);
    assert_eq!(
        (u.current_order, u.last_order, u.current_action),
        (-1, 700, -1)
    );
}

#[test]
fn a_work_notify_sets_the_order_and_redraws_a_timeout_left_by_order_711() {
    let mut u = UnitAi {
        current_order: BUILD_ORDER,
        current_action: BUILD_ACTION,
        current_target: 10028,
        adjusted_idle_timeout: 750,
        idle_timeout: 3000,
        secondary_timer: 900,
        ..idle(8)
    };
    let mut r = rng();
    u.notify_work(
        HUNT_ORDER,
        HUNT_ACTION,
        Some((9365, 15)),
        [24.5, 97.5, 0.0],
        true,
        &mut r,
    )
    .unwrap();
    assert_eq!(
        (u.current_order, u.current_action, u.current_target, r.draws),
        (HUNT_ORDER, HUNT_ACTION, 9365, 0)
    );
    u.current_order = 711;
    u.notify_work(
        HUNT_ORDER,
        HUNT_ACTION,
        Some((9365, 15)),
        [24.5, 97.5, 0.0],
        true,
        &mut r,
    )
    .unwrap();
    assert_eq!((r.draws, u.secondary_timer), (1, 750));
    assert_ne!(u.adjusted_idle_timeout, 750);
}

#[test]
fn a_defending_unit_beyond_five_tiles_of_its_state_position_stops_to_walk_back() {
    // records/425906646's update 1404: player 8's unit on the defend stance (1); within 5
    // tiles of its state position (from its footprint) it goes on to the retarget check
    let defender = |x: f32| {
        let u = UnitAi {
            stance: 1,
            current_order: -1,
            current_action: 0x25a,
            current_target: 4321,
            state_position: [10.0, 10.0],
            retarget_timer: u32::MAX,
            ..idle(13)
        };
        let at = Surroundings {
            position: [x, 10.0, 0.0],
            size: [0.5, 0.5],
            ..Surroundings::default()
        };
        (u, at)
    };
    let (mut u, at) = defender(15.4);
    let mut fx = Vec::new();
    let mut r = rng();
    assert!(
        u.retarget_check(0, false, &at, &mut r, &mut finds_nothing(), &mut fx)
            .is_ok()
    );
    assert_eq!(fx, Vec::<Effect>::new());
    // 5.1 tiles off its footprint: it stops, forgets its target and walks back to its state
    // position (FUN_140c5e7d0), then goes on to the retarget check (its timer not used up)
    let (mut u, at) = defender(15.6);
    assert_eq!(
        u.retarget_check(0, false, &at, &mut r, &mut finds_nothing(), &mut fx),
        Ok(false)
    );
    assert_eq!(
        fx,
        vec![
            Effect::Stop,
            Effect::Move(AiMove {
                point: [10.0, 10.0, 0.0],
                range: 1.0,
                force: true,
                walk: false,
            })
        ]
    );
    assert_eq!((u.current_action, u.current_target), (MOVE_ACTION, -1));
    assert_eq!(u.last_target, 4321);
}

#[test]
fn retarget_waits_for_the_timer() {
    let mut u = idle(17);
    let mut r = rng();
    let _ = u
        .update_idle(
            50,
            899,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!((u.retarget_timer, r.draws), (900, 0));
    let _ = u
        .update_idle(
            50,
            900,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    // the first rand15 after seed 0 is 7241
    assert_eq!(
        (u.last_retarget_time, u.retarget_timer),
        (900, 750 + 7241 % 500)
    );
}

#[test]
fn a_last_retarget_ahead_of_the_world_time_wraps_and_retargets() {
    let mut u = UnitAi {
        last_retarget_time: 400,
        ..idle(15)
    };
    let mut r = rng();
    let _ = u
        .update_idle(
            50,
            100,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!((u.last_retarget_time, r.draws), (100, 1));
}

#[test]
fn villagers_and_livestock_never_retarget() {
    for t in [8, 19] {
        let mut u = idle(t);
        let mut r = rng();
        let _ = u
            .update_idle(
                50,
                5000,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!((u.retarget_timer, r.draws), (900, 0), "type {t}");
    }
}

#[test]
fn the_look_around_timeout_is_redrawn_once_exceeded() {
    let mut u = UnitAi {
        retarget_disabled: 1,
        ..idle(8)
    };
    let mut r = rng();
    let _ = u
        .update_idle(
            1000,
            0,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!((u.look_around_timer, r.draws), (1000, 0));
    let _ = u
        .update_idle(
            1,
            0,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!(
        (u.look_around_timer, u.look_around_timeout),
        (0, 4500 + 7241 % 1000)
    );
}

#[test]
fn retarget_draws_before_look_around() {
    let mut u = UnitAi {
        look_around_timer: 1000,
        ..idle(13)
    };
    let mut r = rng();
    let _ = u
        .update_idle(
            1,
            900,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!(u.retarget_timer, 750 + 7241 % 500);
    assert_eq!(u.look_around_timeout, 4500 + 25269 % 1000);
}

#[test]
fn the_idle_timer_restarts_after_its_timeout_and_the_secondary_timer_stops_at_0() {
    let mut u = UnitAi {
        secondary_timer: 750,
        ..idle(8)
    };
    let mut r = rng();
    let _ = u
        .update_idle(
            3001,
            0,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!((u.idle_timer, u.secondary_timer), (0, 0));
}

#[test]
fn non_idle_units_and_unknown_types_are_unsupported() {
    let mut r = rng();
    let mut busy = UnitAi {
        current_action: 600,
        ..idle(17)
    };
    assert!(
        busy.update_idle(
            1,
            0,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search
        )
        .is_err()
    );
    // a type the slice does not model
    assert!(
        idle(99)
            .update_idle(
                1,
                0,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search
            )
            .is_err()
    );
}

#[test]
fn a_new_building_unit_ai_draws_its_three_timers() {
    // UnitAIModule's constructor: last retarget r % 500, adjusted idle timeout r % 1500 + 2250,
    // retarget timer 750 + r % 500, in that order
    let mut r = rng();
    let ai = UnitAi::constructed(17, &placed([30.0, 88.0, 0.0], 3), &mut r).unwrap();
    let mut expected = rng();
    let (a, b, c) = (expected.rand15(), expected.rand15(), expected.rand15());
    assert_eq!(r.draws, 3);
    assert_eq!(
        ai,
        UnitAi {
            ai_type: 17,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            target_type: -1,
            target_point: [-1.0; 3],
            desired_distance: 2.0,
            defend_target: -1,
            last_order: -1,
            last_action: -1,
            last_target: -1,
            last_target_type: -1,
            idle_timer: 0,
            adjusted_idle_timeout: b % 1500 + 2250,
            idle_timeout: 3000,
            secondary_timer: 750,
            look_around_timer: 0,
            look_around_timeout: 1000,
            last_retarget_time: a % 500,
            retarget_timer: 750 + c % 500,
            stance: 0,
            formation_type: 0,
            state_position: [30.0, 88.0],
            retarget_disabled: 0,
            alert_mode: false,
            time_since_enemy: 0.0,
            patrol_path: None,
            patrol_waypoint: 0,
            stop_after_kill: false,
            home: None,
            hunted: None,
            order_priority: -1,
            last_update_time: 0,
            orders: Vec::new(),
            building_orders: Vec::new(),
            notifies: Vec::new(),
            notify_capacity: 10,
            processing: false,
            buffered: Vec::new(),
            commanded: false,
            attackers: Vec::new(),
            order_history: Vec::new(),
            retries: Vec::new(),
            best_this_update: -1,
        }
    );
}

/// The artifact's unit AI (14, `TribeArtifactUnitAIModule` `FUN_140c11340`, the rock church's):
/// the base's three draws, then `FUN_140c41bc0(5000)`: its idle timeout 5000 and a fourth
/// draw for its adjusted one (`r % 2500 + 3750`), stance 0, five notify slots. The recording
/// 426048706 holds rock churches of base 5000 with adjusted ones of 3881 and 6012.
#[test]
fn a_new_artifact_unit_ai_draws_four_timers() {
    let mut r = rng();
    let ai = UnitAi::constructed(ARTIFACT_AI, &placed([136.0, 100.0, 0.0], 1), &mut r).unwrap();
    let mut expected = rng();
    let (a, _, c, d) = (
        expected.rand15(),
        expected.rand15(),
        expected.rand15(),
        expected.rand15(),
    );
    assert_eq!(r.draws, 4);
    assert_eq!(
        (
            ai.ai_type,
            ai.last_retarget_time,
            ai.retarget_timer,
            ai.idle_timeout,
            ai.adjusted_idle_timeout,
            ai.stance,
            ai.notify_capacity,
        ),
        (
            ARTIFACT_AI,
            a % 500,
            750 + c % 500,
            5000,
            d % 2500 + 3750,
            0,
            5
        )
    );
}

/// What the artifact's tests answer: the search's answer and the owner of the object found,
/// with the searches asked (their radius, classes and reach test) and the unit's
/// under-attack value.
struct ArtifactSide {
    found: Found,
    owner: Option<usize>,
    searches: Vec<(f32, Vec<i32>, bool)>,
    under_attack: bool,
}

impl ArtifactSide {
    fn finding(id: i32, players: u32, owner: Option<usize>) -> Self {
        Self {
            found: Found {
                id,
                players,
                distance: -1,
                position: Some([140.0, 100.0, 0.0]),
                ..Found::NONE
            },
            owner,
            searches: Vec::new(),
            under_attack: false,
        }
    }
}

impl UnitSide for ArtifactSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError> {
        let reach = match search.pick {
            Pick::Closest { reach, .. } => reach,
            _ => true,
        };
        self.searches
            .push((search.radius, search.classes.to_vec(), reach));
        Ok(self.found)
    }
    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Ok(Attacked {
            attacks: false,
            posted: Vec::new(),
        })
    }
    fn set_under_attack(&mut self, value: bool) {
        self.under_attack = value;
    }
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Ok(None)
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Ok(None)
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Ok(None)
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
        Ok(self.owner.map(|owner| UnitSeen {
            id,
            owner,
            ..UnitSeen::default()
        }))
    }
}

/// A scout's world: the points its two searches find, and whether it made the scout map.
#[derive(Default)]
struct ScoutSide {
    unexplored: Option<[i32; 2]>,
    beyond: Option<[i32; 2]>,
    mapped: bool,
}

impl UnitSide for ScoutSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Err(GenieError::unsupported("no attack here"))
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Ok(self.unexplored)
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Ok(self.beyond)
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        self.mapped = true;
        Ok(())
    }
}

/// The base handler's case 0x202 for the auto scout ([`UnitAi::seek_unexplored`]): a point
/// its search finds it explores (the explore's action value the last, its own back) and
/// processing breaks; finding none, on the auto scout order the last search's point is
/// explored the same way, and with none left the scout map is made; on another order the
/// target goes and the unit stops. Another explorer's search, a scout order's that finds
/// none and a computer player's AI told of it are not modelled.
#[test]
fn an_auto_scout_explores_what_its_searches_find_or_maps_what_it_saw() {
    let at = Surroundings {
        id: 20,
        state: 2,
        ..Surroundings::default()
    };
    let scout = |order| UnitAi {
        current_order: order,
        current_action: AUTO_SCOUT_ACTION,
        current_target: 30,
        ..idle(2)
    };
    let seek = |u: &mut UnitAi, side: &mut ScoutSide, computer| {
        let mut fx = Vec::new();
        let r = u.seek_unexplored(&at, computer, &mut rng(), side, &mut fx);
        (r, fx)
    };
    let mut u = scout(AUTO_SCOUT_ORDER);
    let mut side = ScoutSide {
        unexplored: Some([5, 6]),
        ..ScoutSide::default()
    };
    let (r, fx) = seek(&mut u, &mut side, false);
    assert_eq!((r, fx), (Ok(BREAK), vec![Effect::Explore([5.0, 6.0])]));
    assert_eq!(
        (u.current_action, u.last_action),
        (AUTO_SCOUT_ACTION, EXPLORE_ACTION)
    );
    let mut u = scout(AUTO_SCOUT_ORDER);
    let mut side = ScoutSide {
        beyond: Some([7, 8]),
        ..ScoutSide::default()
    };
    let (r, fx) = seek(&mut u, &mut side, false);
    assert_eq!((r, fx), (Ok(BREAK), vec![Effect::Explore([7.0, 8.0])]));
    assert_eq!(
        (u.current_action, u.last_action),
        (AUTO_SCOUT_ACTION, AUTO_SCOUT_ACTION)
    );
    let mut u = scout(AUTO_SCOUT_ORDER);
    let mut side = ScoutSide::default();
    assert_eq!(seek(&mut u, &mut side, false), (Ok(BREAK), vec![]));
    assert!(side.mapped);
    let mut u = scout(AUTO_SCOUT_ORDER);
    assert!(seek(&mut u, &mut ScoutSide::default(), true).0.is_err());
    let mut u = scout(-1);
    assert_eq!(seek(&mut u, &mut ScoutSide::default(), false).0, Ok(GO_ON));
    assert_eq!((u.current_target, u.current_action), (-1, -1));
    let mut u = scout(ai_kinds::ORDER_SCOUT);
    assert!(seek(&mut u, &mut ScoutSide::default(), false).0.is_err());
    let mut u = UnitAi {
        current_action: EXPLORE_ACTION,
        ..scout(-1)
    };
    assert!(seek(&mut u, &mut ScoutSide::default(), false).0.is_err());
}

/// A Gaia rock church (MarvinBlue/426048706's 7108: line of sight 5, no speed) and what it
/// stands on.
fn rock_church() -> (UnitAi, Surroundings) {
    let ai = UnitAi {
        ai_type: ARTIFACT_AI,
        current_order: -1,
        current_action: -1,
        current_target: -1,
        idle_timeout: 5000,
        adjusted_idle_timeout: 3881,
        retarget_timer: 1224,
        ..UnitAi::default()
    };
    let at = Surroundings {
        position: [136.0, 100.0, 0.0],
        size: [2.0, 2.0],
        los: 5.0,
        search_radius: 5.0,
        player: 0,
        state: 2,
        class: 1,
        kind: 80,
        ..Surroundings::default()
    };
    (ai, at)
}

/// The artifact's processIdle (`FUN_140c1e410`): `ClosestObject` over its cared classes from
/// its place within its line of sight and a tile, without the reach test for a unit that
/// does not move; the object found takes it over for its owner (slot 0x70, 6) while the
/// lists hold one player's objects at most; two players, none found, the world without it
/// or its own owner's leave it (5).
#[test]
fn the_artifact_goes_to_the_owner_of_the_closest_object_one_player_sees() {
    let (mut ai, at) = rock_church();
    let mut side = ArtifactSide::finding(500, 1, Some(2));
    let mut fx = Vec::new();
    let took = ai.process_idle(&at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(took, Ok(true));
    assert_eq!(fx, [Effect::ChangeOwner(2)]);
    assert_eq!(side.searches, [(6.0, ARTIFACT_CARED.to_vec(), false)]);
    for (players, id, owner) in [
        (2, 500, Some(2)),
        (1, -1, Some(2)),
        (1, 500, None),
        (0, 500, Some(0)),
    ] {
        let mut side = ArtifactSide::finding(id, players, owner);
        let mut fx = Vec::new();
        let took = ai.process_idle(&at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!((took, fx), (Ok(false), vec![]));
    }
    // a moving one asks for the reach test
    let walking = Surroundings { speed: 0.8, ..at };
    let mut side = ArtifactSide::finding(-1, 0, None);
    let _ = ai.process_idle(&walking, false, &mut rng(), &mut side, &mut Vec::new());
    assert!(side.searches[0].2);
}

/// The artifact's slot 73 (`FUN_140c21d40`) in its full update: its processIdle every
/// update; a takeover forgets its target, stops its action (slot 0xb0 with 1) after the
/// ownership change, and its idle timer is 0.
#[test]
fn the_artifact_s_update_hands_it_over_then_stops() {
    let (mut ai, at) = rock_church();
    ai.idle_timer = 900;
    let mut side = ArtifactSide::finding(500, 1, Some(3));
    let fx = ai
        .update(100, 5000, false, &at, &mut rng(), &mut side)
        .unwrap();
    assert_eq!(fx, [Effect::ChangeOwner(3), Effect::Stop]);
    assert_eq!((ai.current_target, ai.idle_timer), (-1, 0));
    // nobody near: nothing, the idle timer runs
    let (mut ai, at) = rock_church();
    let mut side = ArtifactSide::finding(-1, 0, None);
    let fx = ai
        .update(100, 5000, false, &at, &mut rng(), &mut side)
        .unwrap();
    assert_eq!(fx, []);
    assert_eq!(ai.idle_timer, 100);
}

/// The artifact's notify handler (`FUN_140c21f70`): the kinds it lets be go on; a unit
/// attacking it joins its attackers and puts it under attack; its own action's completion or
/// failure (with an order, the order's action) and any kind it does not know end in its
/// stop; a moved target it still sees goes on, one it does not see ends.
#[test]
fn the_artifact_s_notifies_stop_it_or_go_on() {
    let run = |ai: &mut UnitAi, kind: i32, p: [i32; 3], at: &Surroundings| {
        let n = Notify {
            caller: 500,
            recipient: 7108,
            kind,
            p,
        };
        let mut side = ArtifactSide::finding(-1, 0, None);
        let mut fx = Vec::new();
        let r = ai.handle(n, 0, at, false, &mut rng(), &mut side, &mut fx);
        (r, fx, side.under_attack)
    };
    let (mut ai, at) = rock_church();
    for kind in [
        500,
        0x1f7,
        0x1ff,
        0x202,
        OWNER_CHANGED,
        0x25d,
        0x268,
        699,
        0x2be,
    ] {
        assert_eq!(run(&mut ai, kind, [0; 3], &at), (Ok(GO_ON), vec![], false));
    }
    assert_eq!(run(&mut ai, 600, [0; 3], &at), (Ok(GO_ON), vec![], true));
    assert_eq!(ai.attackers, [500]);
    ai.current_action = 0x262;
    ai.current_target = 9;
    assert_eq!(
        run(&mut ai, ACTION_COMPLETED, [0x261, 0, 0], &at),
        (Ok(GO_ON), vec![], false)
    );
    assert_eq!(
        run(&mut ai, ACTION_FAILED, [0x262, 0, 0], &at),
        (Ok(GO_ON), vec![Effect::Stop], false)
    );
    assert_eq!((ai.current_action, ai.current_target), (-1, -1));
    // with an order: its action's (action + 100 = order)
    ai.current_order = 0x2c6;
    ai.current_action = 0x262;
    assert_eq!(
        run(&mut ai, ACTION_COMPLETED, [0x1, 0, 0], &at),
        (Ok(GO_ON), vec![Effect::Stop], false)
    );
    let unknown = run(&mut ai, 0x210, [0; 3], &at);
    assert_eq!(unknown, (Ok(GO_ON), vec![Effect::Stop], false));
    // a moved target it sees goes on; one it does not see stops it
    ai.current_target = 9;
    let seen = Surroundings {
        target: Some(Seen {
            sight: 0xf,
            ..Seen::default()
        }),
        ..at.clone()
    };
    assert_eq!(
        run(&mut ai, TARGET_MOVED, [0; 3], &seen),
        (Ok(GO_ON), vec![], false)
    );
    let explored = Surroundings {
        target: Some(Seen {
            sight: 0x80,
            ..Seen::default()
        }),
        ..at
    };
    assert_eq!(
        run(&mut ai, TARGET_MOVED, [0; 3], &explored),
        (Ok(GO_ON), vec![Effect::Stop], false)
    );
}

/// The tower's unit AI (15, `TribeTowerUnitAIModule` `FUN_140c12960`, a town centre's with
/// its volley missile): the base's three draws, then `FUN_140c41bc0(1000)`: its idle timeout
/// 1000 and a fourth draw for its adjusted idle timeout (`r % 500 + 750`). records/428759682
/// update 2019: the town centre 8768 a 621's completion makes holds base 1000, adjusted 818.
#[test]
fn a_new_tower_unit_ai_draws_four_timers() {
    let mut r = rng();
    let ai = UnitAi::constructed(TOWER_AI, &placed([54.0, 147.0, 0.0], 52), &mut r).unwrap();
    let mut expected = rng();
    let (a, _, c, d) = (
        expected.rand15(),
        expected.rand15(),
        expected.rand15(),
        expected.rand15(),
    );
    assert_eq!(r.draws, 4);
    assert_eq!(
        (
            ai.ai_type,
            ai.last_retarget_time,
            ai.retarget_timer,
            ai.idle_timeout,
            ai.adjusted_idle_timeout,
            ai.stance,
        ),
        (TOWER_AI, a % 500, 750 + c % 500, 1000, d % 500 + 750, 0)
    );
}

/// A new object of class `class` at `position`, of master 83 (its own copy), a human
/// player's in the Dark Age with posture 0.
fn placed(position: [f32; 3], class: i16) -> NewObject {
    NewObject {
        position,
        master: 83,
        copy_id: 83,
        class,
        ..NewObject::default()
    }
}

/// The base's three draws (`0x140c2e080`) and, for the types whose constructor sets its own
/// idle timeout (`FUN_140c41bc0`), slot 0x1a8's fourth (`0x140c41e00`: `r % (t / 2) + 3 t /
/// 4`); the hunted animal's slot 0x1a8 (`FUN_140c17170`) is 5/4 of its timeout without a
/// draw. Each type's stance (`+0x1a9`) and notify capacity (5 for 0xe and 0x13).
#[test]
fn each_unit_ai_constructor_draws_its_timers_and_sets_its_stance() {
    // (type, class, idle timeout, fourth draw, stance, notify capacity)
    let types: [(u32, i16, u32, bool, u8, usize); 17] = [
        (2, 9, 4000, false, 3, 10),
        (3, 10, 3000, false, 0, 10),
        (BOAR_AI, 10, 3000, false, 0, 10),
        (5, 10, 6000, true, 0, 10),
        (6, 0, 3000, false, 3, 10),
        (7, 0, 3000, false, 3, 10),
        (CIVILIAN, 4, 3000, false, 0, 10),
        (9, 18, 500, true, 1, 10),
        (0xa, 2, 3000, false, 0, 10),
        (0xb, 20, 3000, false, 0, 10),
        (0xc, 21, 3000, false, 0, 10),
        (SOLDIER_AI, 6, 500, true, 0, 10),
        (ARTIFACT_AI, 1, 5000, true, 0, 5),
        (TOWER_AI, 52, 1000, true, 0, 10),
        (0x10, 13, 1000, true, 1, 10),
        (17, 3, 3000, false, 0, 10),
        (0x12, 6, 3000, false, 0, 10),
    ];
    for (ai_type, class, timeout, fourth, stance, capacity) in types {
        let mut r = rng();
        let ai = UnitAi::constructed(ai_type, &placed([12.5, 40.0, 1.0], class), &mut r).unwrap();
        let mut expected = rng();
        let (a, b, c) = (expected.rand15(), expected.rand15(), expected.rand15());
        let adjusted = if fourth {
            expected.rand15() % (timeout / 2) + timeout * 3 / 4
        } else if ai_type == 2 {
            timeout * 5 / 4
        } else {
            b % 1500 + 2250
        };
        assert_eq!(r.draws, if fourth { 4 } else { 3 }, "type {ai_type}");
        assert_eq!(
            (
                ai.ai_type,
                ai.last_retarget_time,
                ai.retarget_timer,
                ai.idle_timeout,
                ai.adjusted_idle_timeout,
                ai.stance,
                ai.notify_capacity,
                ai.state_position,
            ),
            (
                ai_type,
                a % 500,
                750 + c % 500,
                timeout,
                adjusted,
                stance,
                capacity,
                [12.5, 40.0],
            ),
            "type {ai_type}"
        );
    }
    let mut r = rng();
    let livestock = UnitAi::constructed(0x13, &placed([1.0; 3], 58), &mut r).unwrap();
    assert_eq!(
        (r.draws, livestock.stance, livestock.notify_capacity),
        (3, 1, 5)
    );
}

/// `FUN_141215560` has no constructor for the types outside 2..=0x13: such an object gets
/// no unit AI and nothing is drawn.
#[test]
fn a_unit_ai_type_without_a_constructor_makes_none_and_draws_nothing() {
    for ai_type in [0, 1, 0x14, 99] {
        let mut r = rng();
        assert_eq!(
            UnitAi::constructed(ai_type, &placed([0.0; 3], 6), &mut r),
            None
        );
        assert_eq!(r.draws, 0);
    }
}

/// The hunted animal's constructor (`FUN_140c11920`): its origin is the object's place, its
/// leash the object's slot 0x7b0 (`+0xb74`); the king (class 59) takes stance 0, a deer
/// keeps the base's 3. The boar's (`FUN_140c11720`) origin is its place too, neither
/// furious nor sent home.
#[test]
fn a_new_hunted_animal_and_boar_keep_their_place_as_their_origin() {
    let mut deer = placed([20.0, 30.0, 2.0], 9);
    deer.hunt_range = 7.5;
    let mut r = rng();
    let ai = UnitAi::constructed(2, &deer, &mut r).unwrap();
    assert_eq!(
        (ai.hunted, ai.stance, ai.home),
        (
            Some(Hunted {
                origin: [20.0, 30.0, 2.0],
                leash: 7.5,
            }),
            3,
            None
        )
    );
    let king = UnitAi::constructed(2, &placed([0.0; 3], CLASS_KING), &mut rng()).unwrap();
    assert_eq!(king.stance, 0);
    let boar = UnitAi::constructed(BOAR_AI, &placed([5.0, 6.0, 0.5], 10), &mut rng()).unwrap();
    assert_eq!(
        (boar.home, boar.hunted, boar.stance),
        (
            Some(Home {
                origin: [5.0, 6.0, 0.5],
                furious: false,
                go_home: false,
            }),
            None,
            0
        )
    );
}

/// The soldier's constructor (`FUN_140c12300`): a computer player's idle timeout is 1500
/// (the fourth draw `r % 750 + 1125`); its stance by the rules of [`soldier_stance`].
#[test]
fn a_new_soldier_takes_its_stance_from_its_master_and_owner() {
    let mut computer = placed([0.0; 3], 6);
    computer.computer = true;
    computer.posture = 1;
    let mut r = rng();
    let ai = UnitAi::constructed(SOLDIER_AI, &computer, &mut r).unwrap();
    let mut expected = rng();
    let _ = (expected.rand15(), expected.rand15(), expected.rand15());
    assert_eq!(
        (ai.idle_timeout, ai.adjusted_idle_timeout, ai.stance),
        (1500, expected.rand15() % 750 + 1125, 1)
    );
    let stance = |f: &dyn Fn(&mut NewObject)| {
        let mut new = placed([0.0; 3], 6);
        f(&mut new);
        soldier_stance(&new)
    };
    // the owner's posture, its low byte
    assert_eq!(stance(&|_| {}), 0);
    assert_eq!(stance(&|n| n.posture = 0x101), 1);
    // the scout of the Dark Age stands its ground: flag 0x10, its own copy, age 0 (or -0)
    assert_eq!(stance(&|n| n.object_flags = 0x10), 2);
    assert_eq!(
        stance(&|n| {
            n.object_flags = 0x10;
            n.age = -0.0;
        }),
        2
    );
    assert_eq!(
        stance(&|n| {
            n.object_flags = 0x10;
            n.age = 1.0;
        }),
        0
    );
    assert_eq!(
        stance(&|n| {
            n.object_flags = 0x10;
            n.age = f32::NAN;
        }),
        0
    );
    assert_eq!(
        stance(&|n| {
            n.object_flags = 0x10;
            n.copy_id = 448;
        }),
        0
    );
    assert_eq!(
        stance(&|n| {
            n.object_flags = 0x10;
            n.computer = true;
        }),
        0
    );
    // the demolition ships defend
    for master in DEFENSIVE_MASTERS {
        assert_eq!(
            stance(&|n| {
                n.master = master;
                n.copy_id = master;
            }),
            1
        );
    }
    assert_eq!(
        stance(&|n| {
            n.master = 527;
            n.computer = true;
        }),
        0
    );
    // a human player's petards and class 64 do not attack; a computer's take its posture
    assert_eq!(stance(&|n| n.class = 35), 3);
    assert_eq!(stance(&|n| n.class = 64), 3);
    assert_eq!(
        stance(&|n| {
            n.class = 35;
            n.computer = true;
        }),
        0
    );
    // a hero defends: bit 0 or 3, not 3 with 7
    assert_eq!(stance(&|n| n.hero_flag = 1), 1);
    assert_eq!(stance(&|n| n.hero_flag = 8), 1);
    assert_eq!(stance(&|n| n.hero_flag = 0x81), 1);
    assert_eq!(stance(&|n| n.hero_flag = 0x88), 0);
    assert_eq!(stance(&|n| n.hero_flag = 0x10), 0);
    // class 61 does not attack, unless a hero
    assert_eq!(stance(&|n| n.class = 61), 3);
    assert_eq!(
        stance(&|n| {
            n.class = 61;
            n.hero_flag = 1;
        }),
        1
    );
    assert_eq!(
        stance(&|n| {
            n.class = 61;
            n.computer = true;
        }),
        3
    );
}

/// The siege unit's stance (`FUN_140c12040`): a computer player's and the trebuchets' (51,
/// 54) 0, the scorpion's (55) the owner's posture, any other's 1.
#[test]
fn a_new_siege_unit_takes_its_stance_from_its_class_and_owner() {
    let stance = |class: i16, computer: bool, posture: i32| {
        siege_stance(&NewObject {
            class,
            computer,
            posture,
            ..NewObject::default()
        })
    };
    assert_eq!(stance(13, false, 0), 1);
    assert_eq!(stance(13, true, 0), 0);
    assert_eq!(stance(51, false, 1), 0);
    assert_eq!(stance(54, false, 1), 0);
    assert_eq!(stance(55, false, 1), 1);
    assert_eq!(stance(55, false, 0), 0);
    assert_eq!(stance(55, true, 1), 0);
    assert_eq!(stance(6, false, 0), 1);
}

/// [`NewObject::of`]: the master's copy, class, flags and hero flag, and the work range of
/// its last task of action type 6 (0 without one).
#[test]
fn a_new_object_takes_its_hunt_range_from_its_master_s_last_task_of_type_6() {
    let task = |action_type, work_range| crate::masters::Task {
        action_type,
        work_range,
        ..crate::masters::Task::default()
    };
    let made = crate::masters::Master {
        copy_id: 1240,
        class: 9,
        object_flags: 0x30,
        hero_flag: 4,
        tasks: vec![task(6, 2.0), task(7, 9.0), task(6, 3.5), task(5, 1.0)],
        ..crate::masters::Master::default()
    };
    let new = NewObject::of([1.0, 2.0, 3.0], 1239, &made);
    assert_eq!(
        new,
        NewObject {
            position: [1.0, 2.0, 3.0],
            master: 1239,
            copy_id: 1240,
            class: 9,
            object_flags: 0x30,
            hero_flag: 4,
            hunt_range: 3.5,
            ..NewObject::default()
        }
    );
    let none = NewObject::of([0.0; 3], 1, &crate::masters::Master::default());
    assert_eq!(none.hunt_range, 0.0);
}

#[test]
fn an_order_redraws_an_adjusted_idle_timeout_of_750_or_more_and_caps_the_secondary_timer() {
    // FUN_140e6cfc0 for each unit of an INTERACT, MOVE or RETREAT
    let mut u = UnitAi {
        idle_timeout: 3000,
        adjusted_idle_timeout: 2600,
        secondary_timer: 900,
        patrol_path: Some(PatrolPath::default()),
        patrol_waypoint: 3,
        ..idle(13)
    };
    let mut r = rng();
    u.reset_for_order(&mut r);
    assert_eq!(
        (u.adjusted_idle_timeout, u.secondary_timer),
        (7241 % 1500 + 2250, 750)
    );
    // FUN_140c68de0 first: the patrol path and its waypoint go
    assert_eq!((u.patrol_path, u.patrol_waypoint), (None, 0));
    let mut low = UnitAi {
        idle_timeout: 3000,
        adjusted_idle_timeout: 749,
        ..idle(13)
    };
    let mut r = rng();
    low.reset_for_order(&mut r);
    assert_eq!((low.adjusted_idle_timeout, r.draws), (749, 0));
}

/// MarvinBlue/426048731's boar 9732 (type 4) at its origin (23.5, 105.5), as saved after 680.
fn boar() -> UnitAi {
    UnitAi {
        idle_timer: 3000,
        adjusted_idle_timeout: 3005,
        idle_timeout: 3000,
        look_around_timeout: 5426,
        retarget_timer: 1205,
        target_type: -1,
        target_point: [-1.0; 3],
        last_order: -1,
        last_action: -1,
        last_target: -1,
        last_target_type: -1,
        stance: 0,
        home: Some(Home {
            origin: [23.5, 105.5, 0.0],
            ..Home::default()
        }),
        ..idle(4)
    }
}

fn at(position: [f32; 3]) -> Surroundings {
    Surroundings {
        position,
        class: 11,
        ..Surroundings::default()
    }
}

/// A seed whose idle check (a scaled draw, then rand15) wanders, or does not.
#[allow(clippy::maybe_infinite_iter)] // the search ends at the first seed that fits
fn seed_for(wanders: bool) -> Well512 {
    (0..)
        .map(|seed| {
            let mut r = Well512::default();
            r.set_seed(seed);
            r
        })
        .find(|r| {
            let mut e = r.clone();
            let _ = e.below(81);
            (e.rand15() % 7 == 0) == wanders
        })
        .unwrap()
}

#[test]
fn a_boar_at_home_draws_its_leash_and_mostly_stays() {
    // updates 766, 910 and 1039: processIdle draws `0..81` and a rand15, which is not a
    // multiple of 7; nothing moves
    let mut u = boar();
    let mut r = seed_for(false);
    let moved = u.update_idle(
        10,
        0,
        false,
        &at([23.5, 105.5, 0.0]),
        &mut r,
        &mut no_search,
    );
    assert_eq!(moved, Ok(None));
    assert_eq!((u.idle_timer, u.current_action, r.draws), (0, -1, 2));
}

#[test]
fn a_boar_wanders_up_to_three_tiles_once_in_seven() {
    let mut u = boar();
    let mut r = seed_for(true);
    let mut e = r.clone();
    let _ = e.below(81);
    let _ = e.rand15();
    let (dx, dy) = (e.rand15() % 6, e.rand15() % 6);
    let moved = u
        .update_idle(
            10,
            0,
            false,
            &at([23.5, 105.5, 0.0]),
            &mut r,
            &mut no_search,
        )
        .unwrap()
        .unwrap();
    let point = [dx as f32 - 3.0 + 23.5, dy as f32 - 3.0 + 105.5, 0.0];
    assert_eq!(
        moved,
        AiMove {
            point,
            range: 0.5,
            force: true,
            walk: true,
        }
    );
    // slot 0x130: the move action, no target, the point; the last action is the old one
    assert_eq!(
        (
            u.current_action,
            u.last_action,
            u.current_target,
            u.target_point
        ),
        (0x262, -1, -1, point)
    );
    assert_eq!(r.draws, 4);
}

#[test]
fn a_boar_away_from_home_goes_back_and_a_furious_one_stays() {
    // more than 2 tiles from its origin and once in 7: home, at its own height
    let mut u = boar();
    let mut r = seed_for(true);
    let moved = u
        .update_idle(
            10,
            0,
            false,
            &at([26.0, 105.5, 0.5]),
            &mut r,
            &mut no_search,
        )
        .unwrap()
        .unwrap();
    assert_eq!(moved.point, [23.5, 105.5, 0.5]);
    assert_eq!(r.draws, 2);
    // beyond the leash (more than 144 + 80, squared) it heads home unless it already does
    let mut far = boar();
    let mut r = seed_for(false);
    let moved = far.update_idle(
        10,
        0,
        false,
        &at([43.5, 105.5, 0.0]),
        &mut r,
        &mut no_search,
    );
    assert_eq!(moved.unwrap().map(|m| m.point), Some([23.5, 105.5, 0.0]));
    assert_eq!(r.draws, 1);
    let mut heading = UnitAi {
        target_point: [23.5, 0.0, 0.0],
        ..boar()
    };
    let mut r = seed_for(false);
    let moved = heading.update_idle(
        10,
        0,
        false,
        &at([43.5, 105.5, 0.0]),
        &mut r,
        &mut no_search,
    );
    assert_eq!((moved, heading.current_action), (Ok(None), -1));
    // furious, or in mode 1, it does not draw at all; without a target it calms down after
    for (furious, mode_1) in [(true, false), (false, true)] {
        let mut u = boar();
        u.home.as_mut().unwrap().furious = furious;
        let mut r = rng();
        let place = Surroundings {
            mode_1,
            ..at([23.5, 105.5, 0.0])
        };
        assert_eq!(
            u.update_idle(10, 0, false, &place, &mut r, &mut no_search),
            Ok(None)
        );
        assert_eq!((r.draws, u.home.unwrap().furious), (0, false));
    }
}

#[test]
fn a_deer_s_adjusted_idle_timeout_is_five_quarters_of_its_idle_timeout() {
    // FUN_140c17170, the deer's and fox's slot 0x1a8: no draw
    let mut u = UnitAi {
        idle_timeout: 4000,
        adjusted_idle_timeout: 3000,
        ..idle(2)
    };
    let mut r = rng();
    u.reset_for_order(&mut r);
    assert_eq!((u.adjusted_idle_timeout, r.draws), (5000, 0));
}

#[test]
fn a_move_forgets_the_target_and_an_order_keeps_its_history() {
    // MOVE of 9185 in 1044: order 710, action 610, no target, the point on the ground
    let mut u = UnitAi {
        current_target: 9365,
        target_type: 15,
        ..idle(13)
    };
    u.notify_move(true, None, [21.947_916, 92.489_586], &mut rng());
    assert_eq!(
        (
            u.current_order,
            u.current_action,
            u.current_target,
            u.target_type
        ),
        (0x2c6, 0x262, -1, -1)
    );
    assert_eq!(
        (
            u.last_order,
            u.last_action,
            u.last_target,
            u.last_target_type
        ),
        (-1, -1, -1, -1)
    );
    assert_eq!(u.target_point, [21.947_916, 92.489_586, 0.0]);
}

#[test]
fn an_idle_transport_ship_retargets_like_a_soldier() {
    // type 11, TribeTransportShipUnitAIModule: base slots 4 and 73, slot 72 0x140c21d20 draws
    // nothing
    let mut u = idle(11);
    let mut r = rng();
    let _ = u
        .update_idle(
            50,
            900,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!(
        (u.last_retarget_time, u.retarget_timer),
        (900, 750 + 7241 % 500)
    );
}

fn deer() -> UnitAi {
    UnitAi {
        idle_timer: 5000,
        adjusted_idle_timeout: 5000,
        idle_timeout: 4000,
        look_around_timeout: 5424,
        retarget_timer: 1212,
        target_type: -1,
        target_point: [-1.0; 3],
        last_order: -1,
        last_action: -1,
        last_target: -1,
        last_target_type: -1,
        hunted: Some(Hunted {
            origin: [31.5, 106.5, 0.0],
            leash: 0.0,
        }),
        ..idle(2)
    }
}

fn deer_at(position: [f32; 3]) -> Surroundings {
    Surroundings {
        position,
        class: 9,
        size: [0.2, 0.2],
        los: 2.0,
        search_radius: 1.0,
        ..Surroundings::default()
    }
}

/// A stream whose next rand15 is a multiple of 7 or not.
#[allow(clippy::maybe_infinite_iter)] // the search ends at the first seed that fits
fn wander_seed(wanders: bool) -> Well512 {
    (0..)
        .map(|seed| {
            let mut r = Well512::default();
            r.set_seed(seed);
            r
        })
        .find(|r| (r.clone().rand15() % 7 == 0) == wanders)
        .unwrap()
}

#[test]
fn a_deer_that_sees_nothing_stays_six_times_in_seven_and_otherwise_wanders() {
    let mut searched = Vec::new();
    let mut nothing = |s: &Search<'_>| {
        searched.push((s.at, s.radius, s.classes.len(), s.size));
        Ok(Found::NONE)
    };
    let mut u = deer();
    let mut r = wander_seed(false);
    let place = deer_at([31.5, 106.5, 0.0]);
    let moved = u
        .update_idle(10, 0, false, &place, &mut r, &mut nothing)
        .unwrap();
    assert_eq!(moved, None);
    // the search reaches the line of sight plus 1, the search radius being shorter
    assert_eq!(searched, vec![([31.5, 106.5], 3.0, 28, [0.2, 0.2])]);
    assert_eq!(r.draws, 1);

    let mut u = deer();
    let mut r = wander_seed(true);
    let mut e = r.clone();
    let _ = e.rand15();
    let step = |n: u32| if 3 <= n { n as i32 - 6 } else { n as i32 + 1 };
    let (dx, dy) = (step(e.rand15() % 6), step(e.rand15() % 6));
    let moved = u
        .update_idle(10, 0, false, &place, &mut r, &mut |_: &Search<'_>| {
            Ok(Found::NONE)
        })
        .unwrap()
        .unwrap();
    assert_eq!(moved.point, [31.5 + dx as f32, 106.5 + dy as f32, 0.0]);
    assert!(dx != 0 && dy != 0 && dx.abs() <= 3 && dy.abs() <= 3);
    assert_eq!(r.draws, 3);
}

#[test]
fn a_deer_away_from_home_goes_back_and_one_beyond_its_leash_at_once() {
    let mut u = deer();
    let mut r = wander_seed(true);
    let away = deer_at([34.5, 106.5, 0.5]);
    let moved = u
        .update_idle(10, 0, false, &away, &mut r, &mut |_: &Search<'_>| {
            Ok(Found::NONE)
        })
        .unwrap()
        .unwrap();
    assert_eq!((moved.point, r.draws), ([31.5, 106.5, 0.5], 1));
    // a fox's leash of 2.5: 3 tiles off (2.8 from its footprint) it goes home without a
    // search or a draw
    let mut u = deer();
    u.hunted.as_mut().unwrap().leash = 2.5;
    let mut r = wander_seed(false);
    let moved = u
        .update_idle(10, 0, false, &away, &mut r, &mut no_search)
        .unwrap()
        .unwrap();
    assert_eq!((moved.point, r.draws), ([31.5, 106.5, 0.5], 0));
}

/// A deer that sees an object runs from it by three times its speed (slot 0x150), forced
/// and running, and holds the move action and the point; an enemy among what it saw sets its
/// alert mode and starts its sighting time again (deer 9655 at update 2626).
#[test]
fn a_deer_that_sees_an_object_evades_it() {
    let mut u = UnitAi {
        time_since_enemy: 12.0,
        ..deer()
    };
    let mut r = wander_seed(false);
    let place = Surroundings {
        speed: 1.2,
        map: [120, 120],
        ..deer_at([31.5, 106.5, 0.0])
    };
    let enemy = |_: &Search<'_>| {
        Ok(Found {
            id: 9183,
            enemies: true,
            position: Some([33.5, 106.5, 0.0]),
            fresh: true,
            players: 0,
            distance: -1,
            retries: [None; 3],
        })
    };
    let moved = u
        .update_idle(10, 0, false, &place, &mut r, &mut enemy.clone())
        .unwrap()
        .unwrap();
    assert_eq!(
        (moved.point, moved.range, moved.force, moved.walk),
        ([31.5 - 1.2 * 3.0, 106.5, 0.0], 0.0, true, false)
    );
    assert_eq!(
        (u.current_action, u.target_point),
        (MOVE_ACTION, moved.point)
    );
    assert_eq!((u.alert_mode, u.time_since_enemy), (true, 0.0));
}

#[test]
fn an_animal_whose_wander_ended_drops_its_move_action_and_turns_idle() {
    let mut u = UnitAi {
        current_action: 0x262,
        idle_timer: 10,
        ..deer()
    };
    let mut r = wander_seed(false);
    let place = deer_at([31.5, 106.5, 0.0]);
    let moved = u
        .update(10, 0, false, &place, &mut r, &mut no_search)
        .unwrap();
    assert_eq!((moved, u.current_action, u.idle_timer), (vec![], -1, 20));
    // while it walks, its timers run and nothing else happens
    let mut u = UnitAi {
        current_action: 0x262,
        ..deer()
    };
    let walking = Surroundings {
        busy: true,
        ..place
    };
    let _ = u
        .update(10, 0, false, &walking, &mut r, &mut no_search)
        .unwrap();
    assert_eq!(
        (u.current_action, u.idle_timer, u.state_position),
        (0x262, 5010, [0.0; 2])
    );
}

/// MarvinBlue/426048731's scout 9183 (soldier AI, type 13) as saved after 821: on its MOVE
/// order with five queued moves, its `MoveTo` about to arrive.
fn scout() -> UnitAi {
    let order = |x: f32, y: f32| QueuedOrder {
        issuer: 1,
        order: MOVE_ORDER,
        priority: 100,
        target: -1,
        target_owner: -1,
        point: [x, y, 0.0],
        range: 0.0,
        shown: [x, y],
        ready: true,
        target_type: -1,
    };
    UnitAi {
        current_order: MOVE_ORDER,
        order_priority: -1,
        current_action: MOVE_ACTION,
        target_type: -1,
        target_point: [93.583_336, 57.791_668, 0.0],
        last_order: MOVE_ORDER,
        last_action: MOVE_ACTION,
        last_target: -1,
        last_target_type: -1,
        stance: 2,
        orders: vec![order(89.09375, 60.677_082), order(91.822_914, 52.489_582)],
        ..idle(13)
    }
}

fn scout_at(interruptible: bool, busy: bool) -> Surroundings {
    Surroundings {
        position: [93.5, 58.25, 0.0],
        busy,
        player: 1,
        class: 12,
        interruptible,
        ..Surroundings::default()
    }
}

#[test]
fn between_turns_an_arrived_move_runs_the_next_queued_order() {
    // update 822: the MoveTo arrived (state 1, which slot 0x2d0 lets go), so FUN_140c68980
    // runs the queue: the first move becomes the AI's (order 710 at priority 100, action 610,
    // its point) and the unit's, forced and walking; the order leaves the queue
    let mut u = scout();
    let fx = u
        .off_turn(&scout_at(true, true), &mut no_search, false, &mut rng())
        .unwrap();
    assert_eq!(
        fx,
        vec![Effect::Move(AiMove {
            point: [89.09375, 60.677_082, 0.0],
            range: 0.0,
            force: true,
            walk: true,
        })]
    );
    assert_eq!(
        (u.current_order, u.order_priority, u.current_action),
        (MOVE_ORDER, 100, MOVE_ACTION)
    );
    assert_eq!(u.target_point, [89.09375, 60.677_082, 0.0]);
    assert_eq!(u.orders.len(), 1);
    assert_eq!(u.orders[0].point, [91.822_914, 52.489_582, 0.0]);
}

#[test]
fn between_turns_a_walking_unit_or_one_with_notifies_keeps_its_queue() {
    let mut walking = scout();
    assert_eq!(
        walking.off_turn(&scout_at(false, true), &mut no_search, false, &mut rng()),
        Ok(vec![])
    );
    assert_eq!(walking.orders.len(), 2);
    let mut told = UnitAi {
        notifies: vec![Notify::own(9183, ACTION_COMPLETED, [MOVE_ACTION, 0, 0])],
        ..scout()
    };
    assert_eq!(
        told.off_turn(&scout_at(true, true), &mut no_search, false, &mut rng()),
        Ok(vec![])
    );
    assert_eq!(told.orders.len(), 2);
    // an order not ready stays and the next one runs
    let mut u = scout();
    u.orders[0].ready = false;
    let _ = u
        .off_turn(&scout_at(true, true), &mut no_search, false, &mut rng())
        .unwrap();
    assert_eq!(u.orders.len(), 1);
    assert_eq!(u.orders[0].point, [89.09375, 60.677_082, 0.0]);
}

#[test]
fn a_completed_move_on_its_order_ends_the_order_and_stops_the_unit() {
    // the base handler's 0x1fa: 610 + 100 = 710, so slot 0xb0 ends the order too; the
    // notifies go with the owner's time; the queue then runs (the unit is no longer busy)
    let mut u = UnitAi {
        orders: vec![],
        notifies: vec![Notify::own(9183, ACTION_COMPLETED, [MOVE_ACTION, 0, 0])],
        ..scout()
    };
    let mut r = rng();
    let fx = u
        .update(
            60,
            900,
            false,
            &scout_at(true, false),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!(fx, vec![Effect::Stop]);
    assert_eq!(
        (u.current_order, u.order_priority, u.current_action),
        (-1, -1, -1)
    );
    assert_eq!((u.last_order, u.last_action), (MOVE_ORDER, MOVE_ACTION));
    assert_eq!(u.state_position, [93.5, 58.25]);
    assert_eq!((u.notifies.len(), u.last_update_time), (0, 60));
}

#[test]
fn a_failed_action_stops_like_a_completed_one_and_an_order_s_own_failure_ends_it() {
    // records/425906646's update 1234: a Gaia animal's own MoveTo failed (state 13, notify
    // 0x1f9); without an order the failure of the current action stops it
    let mut u = UnitAi {
        current_order: -1,
        current_action: MOVE_ACTION,
        current_target: 31234,
        ..UnitAi::default()
    };
    let mut fx = Vec::new();
    let n = Notify::own(9183, ACTION_FAILED, [MOVE_ACTION, 0, 0]);
    assert_eq!(
        u.action_failed(
            n,
            &at([10.0, 10.0, 0.0]),
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut fx
        )
        .unwrap(),
        GO_ON
    );
    assert_eq!((u.current_action, u.last_action), (-1, MOVE_ACTION));
    assert_eq!((u.current_target, u.last_target), (-1, 31234));
    // on its order (610 + 100) the failure ends the order too and sets the state position
    let mut u = UnitAi {
        current_order: MOVE_ORDER,
        current_action: MOVE_ACTION,
        ..UnitAi::default()
    };
    let mut fx = Vec::new();
    let _ = u
        .action_failed(
            n,
            &at([10.0, 12.0, 0.0]),
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut fx,
        )
        .unwrap();
    assert_eq!((u.current_order, u.current_action), (-1, -1));
    assert_eq!(u.state_position, [10.0, 12.0]);
    // another action's failure goes on; a failed attack on the attack order of a unit AI
    // but a boar's is not modelled
    let mut u = UnitAi {
        current_order: -1,
        current_action: 0x25a,
        ..UnitAi::default()
    };
    let _ = u
        .action_failed(
            n,
            &at([10.0, 12.0, 0.0]),
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut fx,
        )
        .unwrap();
    assert_eq!(u.current_action, 0x25a);
    let mut u = UnitAi {
        current_order: 700,
        ..UnitAi::default()
    };
    let attack = Notify::own(9183, ACTION_FAILED, [600, 0, 0]);
    assert!(
        u.action_failed(
            attack,
            &at([10.0, 12.0, 0.0]),
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut fx
        )
        .is_err()
    );
}

/// records/425906646's wolf 29827 (Gaia, a boar's unit AI) in update 3891: its Attack's target
/// went out of sight, which sent it home (`+0x1fd`); the failure's processIdle (slot 0x240)
/// walks it there and breaks off. A boar that stays forgets its target and stops.
#[test]
fn a_boar_s_failed_attack_runs_its_idle_check() {
    let home = |go_home, furious| Home {
        origin: [171.5, 51.5, 0.0],
        furious,
        go_home,
    };
    let boar = |h| UnitAi {
        ai_type: BOAR_AI,
        current_order: -1,
        current_action: 600,
        current_target: 31110,
        home: Some(h),
        ..UnitAi::default()
    };
    let attack = Notify::own(29827, ACTION_FAILED, [600, 0, 0]);
    let mut u = boar(home(true, false));
    let mut fx = Vec::new();
    let r = u.action_failed(
        attack,
        &at([169.5, 50.6, 0.0]),
        false,
        &mut rng(),
        &mut Recorder::default(),
        &mut fx,
    );
    assert_eq!(r, Ok(PRODUCED));
    assert_eq!(first_move(&fx).map(|m| m.point), Some([171.5, 51.5, 0.0]));
    assert_eq!((u.current_action, u.last_action), (MOVE_ACTION, 600));
    assert!(u.home.is_some_and(|h| !h.go_home));
    let mut u = boar(home(false, true));
    let mut fx = Vec::new();
    let r = u.action_failed(
        attack,
        &at([169.5, 50.6, 0.0]),
        false,
        &mut rng(),
        &mut Recorder::default(),
        &mut fx,
    );
    assert_eq!(r, Ok(GO_ON));
    assert_eq!((u.current_target, u.last_target), (-1, 31110));
    assert!(first_move(&fx).is_none());
}

#[test]
fn an_animal_s_completed_wander_becomes_its_last_action() {
    // the fox 9851 in 718: its MoveTo's completion through the hunted animal's handler to
    // the base: no order, its action completed
    let mut u = UnitAi {
        current_action: MOVE_ACTION,
        idle_timer: 0,
        notifies: vec![Notify::own(9851, ACTION_COMPLETED, [MOVE_ACTION, 0, 0])],
        ..deer()
    };
    let mut r = wander_seed(false);
    let fx = u
        .update(
            60,
            0,
            false,
            &deer_at([31.5, 106.5, 0.0]),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!(fx, vec![Effect::Stop]);
    assert_eq!((u.current_action, u.last_action), (-1, MOVE_ACTION));
}

#[test]
fn a_movement_completion_reaches_only_a_moving_unit_ai() {
    let completed = Notify::own(9181, ACTION_COMPLETED, [MOVE_ACTION, 0, 0]);
    let mut builder = UnitAi {
        current_order: BUILD_ORDER,
        current_action: BUILD_ACTION,
        ..idle(8)
    };
    builder.post(completed, 2).unwrap();
    assert_eq!(builder.notifies, Vec::<Notify>::new());
    let mut wanderer = UnitAi {
        current_action: MOVE_ACTION,
        ..deer()
    };
    wanderer.post(completed, 2).unwrap();
    assert_eq!(wanderer.notifies, vec![completed]);
    // a unit not in state 2 tells its AI nothing
    let mut dying = UnitAi {
        current_action: MOVE_ACTION,
        ..deer()
    };
    dying.post(completed, 3).unwrap();
    assert_eq!(dying.notifies, Vec::<Notify>::new());
}

#[test]
fn a_livestock_s_ownership_notify_drops_its_action_value_and_breaks_off() {
    // FUN_140c26ac0: the notify ends the pass; the update's FUN_140c5dc10 clears the queue
    let mut u = UnitAi {
        current_action: MOVE_ACTION,
        notifies: vec![
            Notify::own(9519, OWNER_CHANGED, [2, 0, 0]),
            Notify::own(9519, ACTION_COMPLETED, [MOVE_ACTION, 0, 0]),
        ],
        ..idle(19)
    };
    let mut r = rng();
    let fx = u
        .update(
            60,
            0,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!(fx, vec![]);
    assert_eq!((u.current_action, u.last_action), (-1, MOVE_ACTION));
    assert_eq!((u.notifies.len(), u.last_update_time), (0, 60));
}

/// Slot 0x28: a unit past state 2 takes no order; one in front goes before the first queued
/// order, any other after the last; an order not ready is what `FUN_140c4dfa0` looks for.
#[test]
fn an_order_for_the_queue_goes_after_the_last_or_before_the_first() {
    let order = |target, ready| QueuedOrder {
        target,
        order: ORDER_ON_TARGET,
        ready,
        ..QueuedOrder::default()
    };
    let mut ai = UnitAi::default();
    assert_eq!(ai.queue_order(3, order(1, true), false, false), Ok(false));
    assert_eq!(ai.orders, Vec::<QueuedOrder>::new());
    for (target, front) in [(1, true), (2, false), (3, true)] {
        assert_eq!(
            ai.queue_order(2, order(target, true), false, front),
            Ok(true)
        );
    }
    let targets: Vec<i32> = ai.orders.iter().map(|o| o.target).collect();
    assert_eq!(targets, [3, 1, 2]);
    assert!(!ai.has_unready_order());
    let _ = ai.queue_order(2, order(4, false), false, false).unwrap();
    assert!(ai.has_unready_order());
    assert!(ai.queue_order(2, order(5, true), true, false).is_err());
}

#[test]
fn a_saved_order_and_notify_read_as_the_game_wrote_them() {
    let mut order = [0u8; 52];
    for (i, w) in [1i32, 710, 100, -1, -1].iter().enumerate() {
        order[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    for (i, f) in [91.822_914_f32, 52.489_582, 0.0, 0.0, 91.822_914, 52.489_582]
        .iter()
        .enumerate()
    {
        order[20 + 4 * i..24 + 4 * i].copy_from_slice(&f.to_le_bytes());
    }
    order[44] = 1;
    order[48..52].copy_from_slice(&(-1i32).to_le_bytes());
    let o = QueuedOrder::from_bytes(&order);
    assert_eq!(
        (
            o.issuer,
            o.order,
            o.priority,
            o.point,
            o.range,
            o.ready,
            o.target_type
        ),
        (1, 710, 100, [91.822_914, 52.489_582, 0.0], 0.0, true, -1)
    );
    let mut notify = [0u8; 24];
    for (i, w) in [9851i32, 9851, 506, 610, 0, 0].iter().enumerate() {
        notify[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    assert_eq!(
        Notify::from_bytes(&notify),
        Notify::own(9851, ACTION_COMPLETED, [MOVE_ACTION, 0, 0])
    );
}

/// MarvinBlue/426048731's builders 9175 and 9177 in update 1497: their Builds ended (no
/// actions left) on their build order (702/602) with the house 10028 as their target.
fn builder_done() -> UnitAi {
    UnitAi {
        current_order: BUILD_ORDER,
        current_action: BUILD_ACTION,
        current_target: 10028,
        target_type: 3,
        target_point: [30.0, 88.0, 0.0],
        idle_timeout: 3000,
        ..idle(8)
    }
}

#[test]
fn a_making_building_runs_process_idle_only_without_a_target() {
    // a town centre (type 15, class 3) on its make action: with a target, or within its idle
    // timeout, the idle part does nothing and keeps the timer
    let tc = |target| UnitAi {
        ai_type: 15,
        current_order: ATTACK_ORDER,
        current_action: crate::production::spawn::MAKE_ACTION,
        current_target: target,
        idle_timer: 4000,
        adjusted_idle_timeout: 1080,
        ..UnitAi::default()
    };
    let at = Surroundings {
        class: CLASS_3,
        kind: BUILDING_KIND,
        ranged: true,
        busy: true,
        ..Surroundings::default()
    };
    let run = |u: &mut UnitAi, at: &Surroundings| {
        u.idle_part(
            true,
            at.group,
            at,
            false,
            &mut Well512::default(),
            &mut Recorder::default(),
            &mut Vec::new(),
        )
    };
    let mut u = tc(29826);
    run(&mut u, &at).unwrap();
    assert_eq!(u.idle_timer, 4000);
    let mut u = UnitAi {
        idle_timer: 900,
        ..tc(-1)
    };
    run(&mut u, &at).unwrap();
    assert_eq!(u.idle_timer, 900);
    // past it without a target the timer restarts and processIdle runs: the tower's search
    let mut u = tc(-1);
    assert_eq!(
        run(&mut u, &at),
        Err(GenieError::unsupported("no search here"))
    );
    assert_eq!(u.idle_timer, 0);
    // a building without a missile and a single volley forgets its target, then searches
    let single = Surroundings {
        ranged: false,
        firepower: 1.0,
        ..at
    };
    let mut u = tc(29826);
    assert_eq!(
        run(&mut u, &single),
        Err(GenieError::unsupported("no search here"))
    );
    assert_eq!((u.current_target, u.last_target), (-1, 29826));
    // with units inside it shoots more and keeps it
    let manned = Surroundings {
        firepower: 3.0,
        ..single
    };
    let mut u = tc(29826);
    run(&mut u, &manned).unwrap();
    assert_eq!((u.current_target, u.idle_timer), (29826, 4000));
}

/// The full update with a target on the orders and action values the recordings met (it
/// reads the target generally, whatever the order): a town centre's make action on the
/// attack order (6091, p6's 27171), a wolf's own attack, a building a MOVE took (no action
/// value, its research's 620), a builder and a hunter sent off a footprint, a hunt without
/// an order after `HuntObject`, a villager waiting for its foundation (734/634 with the
/// foundation), a boar moved aside off its attacker (-1/703).
#[test]
fn a_unit_ai_with_a_target_runs_its_update_on_any_order() {
    let at = Surroundings {
        busy: true,
        player: 1,
        state: 2,
        target: Some(Seen {
            state: 2,
            kind: 70,
            owner: 2,
            class: 4,
            ..Seen::default()
        }),
        ..Surroundings::default()
    };
    for (ai_type, order, action) in [
        (15, ATTACK_ORDER, crate::production::spawn::MAKE_ACTION),
        (5, -1, ATTACK_ACTION),
        (17, MOVE_ORDER, -1),
        (17, MOVE_ORDER, 620),
        (CIVILIAN, BUILD_ORDER, MOVE_ASIDE_ACTION),
        (CIVILIAN, HUNT_ORDER, MOVE_ASIDE_ACTION),
        (CIVILIAN, -1, HUNT_ACTION),
        (CIVILIAN, FOUNDATION_ORDER, WAIT_SOLIDIFY_ACTION),
        (BOAR_AI, -1, MOVE_ASIDE_ACTION),
    ] {
        let mut u = UnitAi {
            ai_type,
            current_order: order,
            current_action: action,
            current_target: 14230,
            adjusted_idle_timeout: 9000,
            look_around_timeout: 9000,
            retarget_timer: 90_000,
            home: (ai_type == BOAR_AI).then(Home::default),
            ..UnitAi::default()
        };
        let r = u.update(10, 100, false, &at, &mut rng(), &mut TailSide::default());
        assert_eq!(r, Ok(Vec::new()), "{ai_type} {order} {action}");
        assert_eq!(u.current_target, 14230, "{ai_type} {order} {action}");
    }
}

#[test]
fn the_target_block_keeps_a_standing_target_and_drops_a_gone_one() {
    // 1497: the house exists in state 2, which keeps it
    let standing = Surroundings {
        state: 2,
        target: Some(Seen {
            state: 2,
            kind: 80,
            ..Seen::default()
        }),
        ..Surroundings::default()
    };
    let mut u = builder_done();
    let mut fx = Vec::new();
    u.target_block(
        &standing,
        false,
        &mut Well512::default(),
        &mut Recorder::default(),
        &mut fx,
    )
    .unwrap();
    assert_eq!((u.current_target, &fx[..]), (10028, &[][..]));
    // a target no longer in the world goes (its class and point stay); an attack action's
    // idle timer then takes its timeout
    let gone = Surroundings {
        state: 2,
        ..Surroundings::default()
    };
    let mut u = UnitAi {
        current_action: ATTACK_ACTION,
        ..builder_done()
    };
    u.target_block(
        &gone,
        false,
        &mut Well512::default(),
        &mut Recorder::default(),
        &mut fx,
    )
    .unwrap();
    assert_eq!((u.current_target, u.target_type), (-1, 3));
    assert_eq!(u.idle_timer, 3000);
    // inside another, a unit drops its target before the lookup
    let inside = Surroundings {
        garrisoned: true,
        ..standing
    };
    let mut u = builder_done();
    u.target_block(
        &inside,
        false,
        &mut Well512::default(),
        &mut Recorder::default(),
        &mut fx,
    )
    .unwrap();
    assert_eq!(u.current_target, -1);
}

#[test]
fn a_dead_target_stops_an_action_that_needs_a_live_one() {
    let dead = Surroundings {
        state: 2,
        target: Some(Seen {
            state: 3,
            kind: 70,
            ..Seen::default()
        }),
        ..Surroundings::default()
    };
    // 0x25b needs a live target: the target goes and the action stops
    let mut u = UnitAi {
        current_order: -1,
        current_action: 0x25b,
        ..builder_done()
    };
    let mut fx = Vec::new();
    u.target_block(
        &dead,
        false,
        &mut Well512::default(),
        &mut Recorder::default(),
        &mut fx,
    )
    .unwrap();
    assert_eq!(fx, vec![Effect::Stop]);
    assert_eq!((u.current_target, u.last_target), (-1, 10028));
    assert_eq!((u.current_action, u.last_action), (-1, 0x25b));
    // a hunt goes on at its carcass
    let mut u = UnitAi {
        current_order: HUNT_ORDER,
        current_action: HUNT_ACTION,
        ..builder_done()
    };
    let mut fx = Vec::new();
    u.target_block(
        &dead,
        false,
        &mut Well512::default(),
        &mut Recorder::default(),
        &mut fx,
    )
    .unwrap();
    assert_eq!((u.current_target, fx), (10028, vec![]));
    // an attack whose target died waits while its Attack swings on the kill (state 2); past
    // it the villager's processIdle (the cap, then 5) sends it to the dead target's point; an
    // Attack the slice does not hold is not modelled
    let attack = || UnitAi {
        current_action: ATTACK_ACTION,
        ..builder_done()
    };
    let swinging = Surroundings {
        action_state: Some(2),
        ..dead.clone()
    };
    let mut u = attack();
    let mut fx = Vec::new();
    u.target_block(
        &swinging,
        false,
        &mut Well512::default(),
        &mut Recorder::default(),
        &mut fx,
    )
    .unwrap();
    assert_eq!((u.current_target, fx), (10028, vec![]));
    let over = Surroundings {
        action_state: Some(1),
        ..dead.clone()
    };
    let mut u = attack();
    let mut fx = Vec::new();
    u.target_block(
        &over,
        false,
        &mut Well512::default(),
        &mut Recorder::default(),
        &mut fx,
    )
    .unwrap();
    assert_eq!(
        fx,
        vec![
            Effect::CapHumanOrder(HUMAN_ORDER_CAP),
            Effect::Move(AiMove {
                point: [30.0, 88.0, 0.0],
                range: 0.0,
                force: true,
                walk: false,
            })
        ]
    );
    assert_eq!((u.current_order, u.last_order), (-1, BUILD_ORDER));
    assert!(
        attack()
            .target_block(
                &dead,
                false,
                &mut Well512::default(),
                &mut Recorder::default(),
                &mut Vec::new()
            )
            .is_err()
    );
}

/// Player 1's scout 9183 following the deer 9655 (FOLLOW at 2604): order 712, action 612,
/// the target and its place when FOLLOW ran, the constructor's desired distance.
fn follower(deer: [f32; 3], visible: bool) -> (UnitAi, Surroundings) {
    let u = UnitAi {
        ai_type: 13,
        current_order: FOLLOW_ORDER,
        current_action: FOLLOW_ACTION,
        current_target: 9655,
        target_type: 9,
        target_point: [115.6998, 39.299_213, 0.0],
        desired_distance: 2.0,
        stance: 2,
        ..UnitAi::default()
    };
    let at = Surroundings {
        position: [110.0, 40.0, 0.0],
        size: [0.2, 0.2],
        state: 2,
        busy: true,
        class: 12,
        target: Some(Seen {
            state: 2,
            kind: 70,
            class: 9,
            position: deer,
            radius: [0.2, 0.2],
            visible,
            ..Seen::default()
        }),
        ..Surroundings::default()
    };
    (u, at)
}

#[test]
fn the_follow_order_moves_to_a_target_that_left_its_point() {
    // 2652: the deer is more than a tile off the point in y; beyond 2 the AI moves to it
    let deer = [114.75516, 40.34795, 0.0];
    let (mut u, at) = follower(deer, true);
    let mut fx = Vec::new();
    assert!(u.follow(&at, &mut fx).unwrap());
    assert_eq!(
        fx,
        vec![Effect::MoveToObject {
            target: 9655,
            state: 2,
            range: 2.0
        }]
    );
    assert_eq!(
        (u.current_action, u.last_action),
        (MOVE_ACTION, FOLLOW_ACTION)
    );
    assert_eq!(
        (u.current_target, u.last_target, u.target_type),
        (9655, 9655, 9)
    );
    assert_eq!(u.target_point, deer);
    // less than a tile off on both axes it stays
    let (mut u, at) = follower([115.0, 40.2, 0.0], true);
    let mut fx = Vec::new();
    assert!(!u.follow(&at, &mut fx).unwrap());
    assert_eq!(fx, Vec::<Effect>::new());
    assert_eq!(u.target_point, [115.6998, 39.299_213, 0.0]);
}

#[test]
fn the_follow_order_ends_on_a_target_out_of_sight() {
    let (mut u, at) = follower([114.75516, 40.34795, 0.0], false);
    let mut fx = Vec::new();
    assert!(u.follow(&at, &mut fx).unwrap());
    assert_eq!(fx, vec![Effect::Stop]);
    assert_eq!(
        (u.current_order, u.current_action, u.current_target),
        (-1, -1, -1)
    );
    assert_eq!((u.last_target, u.last_action), (9655, FOLLOW_ACTION));
}

/// The base handler's case 0x1fc (a `MoveTo`'s moved target): nothing without a saved
/// patrol; with one processIdle runs (slot 0x240 with 1; this Gaia soldier only caps).
#[test]
fn a_moved_target_notify_runs_process_idle_with_a_saved_patrol() {
    let (mut u, at) = follower([115.0, 40.2, 0.0], true);
    let n = Notify::own(9183, TARGET_MOVED, [0, 0, 0]);
    let (mut r, mut side) = (rng(), Recorder::default());
    let mut fx = Vec::new();
    assert_eq!(
        u.base_notify(n, 0, &at, false, &mut r, &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!(fx, vec![]);
    // off its follow order (whose last human order time stays) the idle check caps it
    u.patrol_path = Some(PatrolPath::default());
    u.current_order = -1;
    assert_eq!(
        u.base_notify(n, 0, &at, false, &mut r, &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
}

/// The scout 9185 in update 1518: idle (its MOVE done), stance 2 at its state position.
fn idle_scout() -> (UnitAi, Surroundings) {
    let u = UnitAi {
        idle_timer: 504,
        adjusted_idle_timeout: 503,
        stance: 2,
        state_position: [21.947_916, 92.489_586],
        ..idle(13)
    };
    let at = Surroundings {
        position: [21.947_916, 92.489_586, 0.0],
        player: 2,
        class: 12,
        size: [0.2, 0.2],
        los: 4.0,
        search_radius: 4.0,
        state: 2,
        kind: 70,
        master: 448,
        ..Surroundings::default()
    };
    (u, at)
}

#[test]
fn an_idle_soldier_caps_its_human_order_time_and_searches_for_something_to_attack() {
    let (mut u, at) = idle_scout();
    let mut searched = Vec::new();
    let mut closest = |s: &Search<'_>| {
        searched.push((s.radius, s.classes.len(), s.pick));
        Ok(Found::NONE)
    };
    let fx = u
        .update(28, 0, false, &at, &mut rng(), &mut closest)
        .unwrap();
    assert_eq!(fx, vec![Effect::CapHumanOrder(5000)]);
    // the search radius: its line of sight plus 1; Gaia's objects too (not a scenario)
    assert!(matches!(
        searched[..],
        [(5.0, 39, Pick::Attack { lowest: 0, .. })]
    ));
    assert_eq!(u.idle_timer, 0);
    // something found would be attacked, which is not modelled
    let (mut u, at) = idle_scout();
    let mut found = |_: &Search<'_>| {
        Ok(Found {
            id: 9731,
            enemies: false,
            position: Some([0.0; 3]),
            fresh: true,
            players: 0,
            distance: -1,
            retries: [None; 3],
        })
    };
    assert!(u.update(28, 0, false, &at, &mut rng(), &mut found).is_err());
    // away from its state position, a stance-2 soldier walks back ("I'm in Defend or
    // StandGround, returning to state pos.", FUN_140c5e7d0), which its processIdle does not
    // count as an action (5)
    let (mut u, at) = idle_scout();
    u.state_position = [25.0, 92.5];
    let mut none = |_: &Search<'_>| Ok(Found::NONE);
    let fx = u.update(28, 0, false, &at, &mut rng(), &mut none).unwrap();
    assert_eq!(
        fx,
        vec![
            Effect::CapHumanOrder(5000),
            Effect::Move(AiMove {
                point: [25.0, 92.5, 0.0],
                range: 1.0,
                force: true,
                walk: false,
            })
        ]
    );
    assert_eq!((u.last_action, u.current_action), (-1, MOVE_ACTION));
}

#[test]
fn a_gaia_soldier_that_takes_owners_by_sight_does_not_search() {
    let (mut u, at) = idle_scout();
    let gaia = Surroundings {
        player: 0,
        capture: 1,
        ..at
    };
    let fx = u
        .update(28, 0, false, &gaia, &mut rng(), &mut no_search)
        .unwrap();
    assert_eq!(fx, vec![Effect::CapHumanOrder(5000)]);
}

/// Player 2's town centre 9156 in MarvinBlue/426048731's update 1506.
const TC: Attacker = Attacker {
    ai_type: 15,
    player: 2,
    computer: false,
    state: 2,
    held: -1,
    top_action: None,
    master: 109,
    kind: 80,
    class: 3,
    attack_priority: 0,
    missile: 54,
    range: 6.0,
    min_range: 0.0,
    charge_type: 0,
    area_effect: 0.0,
    blast_level: 0,
    los: 8.0,
    position: [29.0, 94.0, 0.0],
    radius: [2.0, 2.0],
    speed: 0.0,
    scenario: false,
    firepower: 0.0,
    capture: 0,
};

/// Gaia's boar 9731 (master 48, class 10).
const BOAR: Target = Target {
    owner: 0,
    class: 10,
    kind: 70,
    master: 48,
    combat_level: 0,
    capture: 0,
    has_ai: true,
    inside: false,
    targeting: true,
    attackable: true,
    desolid: false,
    position: [30.0, 98.0, 0.0],
    radius: [0.5, 0.5],
};

/// A unit AI without an order or a target, in the aggressive stance.
const IDLE: Seeker = Seeker {
    stance: 0,
    state_position: [0.0, 0.0],
    order: -1,
    action: -1,
    target: -1,
    target_type: -1,
    last_target_type: -1,
    patrol_path: false,
    target_kind: None,
};

/// Slot 0xa0 by unit-AI type: the base's always, the building's never, the hunted
/// animal's only with an Attack (or a type 7) in its list, the predator's and lion's by
/// class under attack priority 0 (anything under 1 and 2), the civilian's but classes 1 and
/// 42; the priest's is not modelled.
#[test]
fn each_unit_ai_type_has_its_own_attack_check() {
    let check = |ai_type: u32, top_action: Option<i32>, priority: u8, class: i16| {
        let me = Attacker {
            ai_type,
            top_action,
            attack_priority: priority,
            ..TC
        };
        can_attack(&me, &IDLE, &Target { class, ..BOAR })
    };
    for t in [
        BOAR_AI,
        6,
        7,
        TRADE_AI,
        TRANSPORT_AI,
        FISHING_AI,
        ARTIFACT_AI,
        0x12,
        0x13,
    ] {
        assert_eq!(check(t, None, 0, 42), Ok(true), "{t}");
    }
    assert_eq!(check(BUILDING_AI, None, 0, 4), Ok(false));
    assert_eq!(check(2, None, 0, 4), Ok(false));
    assert_eq!(check(2, Some(9), 0, 4), Ok(true));
    assert_eq!(check(2, Some(7), 0, 4), Ok(true));
    // the predator's list and the lion's: 4 (villagers) yes, 10 no
    assert_eq!(
        (check(3, None, 0, 4), check(3, None, 0, 10)),
        (Ok(true), Ok(false))
    );
    assert_eq!(check(3, None, 2, 10), Ok(true));
    assert_eq!(
        (check(5, None, 0, 9), check(3, None, 0, 9)),
        (Ok(true), Ok(false))
    );
    assert_eq!(check(CIVILIAN, None, 0, 42), Ok(false));
    assert_eq!(check(CIVILIAN, None, 0, 10), Ok(true));
    assert!(check(PRIEST_AI, None, 0, 4).is_err());
}

#[test]
fn a_tower_passes_a_boar_but_not_a_wolf() {
    assert_eq!(can_attack(&TC, &IDLE, &BOAR), Ok(false));
    let wolf = Target {
        combat_level: 4,
        ..BOAR
    };
    assert_eq!(can_attack(&TC, &IDLE, &wolf), Ok(true));
    // Gaia's tower attacks nothing; a Gaia object outside classes 9 and 10 that takes owners
    // by sight neither
    assert_eq!(
        can_attack(&Attacker { player: 0, ..TC }, &IDLE, &wolf),
        Ok(false)
    );
    let sheep = Target {
        class: 58,
        capture: 3,
        ..BOAR
    };
    assert_eq!(can_attack(&TC, &IDLE, &sheep), Ok(false));
    // a type whose attack check is not modelled (the priest's conversion test)
    assert!(
        can_attack(
            &Attacker {
                ai_type: PRIEST_AI,
                ..TC
            },
            &IDLE,
            &BOAR
        )
        .is_err()
    );
    // a tower without a missile is armed only by the firepower of the units inside it
    let bare = Attacker { missile: -1, ..TC };
    let enemy = Target {
        owner: 1,
        class: 6,
        ..BOAR
    };
    assert_eq!(can_attack(&bare, &IDLE, &enemy), Ok(false));
    let manned = Attacker {
        firepower: 2.0,
        ..bare
    };
    assert_eq!(can_attack(&manned, &IDLE, &enemy), Ok(true));
}

/// A scout cavalry (master 448, class 47) of player 1 in `StandGround` at its state position,
/// as records/425906646's 27272 at update 4268.
const SCOUT: Attacker = Attacker {
    ai_type: 13,
    player: 1,
    kind: 70,
    class: 47,
    master: 448,
    missile: -1,
    range: 0.0,
    los: 4.0,
    position: [118.5, 183.5, 0.0],
    radius: [0.4, 0.4],
    speed: 1.2,
    ..TC
};
const STAND_GROUND: Seeker = Seeker {
    stance: 2,
    state_position: [118.5, 183.5],
    ..IDLE
};

#[test]
fn a_soldier_standing_its_ground_attacks_within_its_range_of_its_state_position() {
    let villager = |x: f32| Target {
        owner: 2,
        class: 4,
        master: 83,
        position: [x, 183.5, 0.0],
        radius: [0.2, 0.2],
        ..BOAR
    };
    // the range against it plus 0.5 from the state position to its footprint
    let reach = |s: &Seeker, x: f32| soldier_can_attack(&SCOUT, s, &villager(x));
    assert_eq!(reach(&STAND_GROUND, 119.2), Ok(true));
    assert_eq!(reach(&STAND_GROUND, 119.3), Ok(false));
    // on order 0x2bd anywhere; in Defend 5 more
    let ordered = Seeker {
        order: 0x2bd,
        ..STAND_GROUND
    };
    assert_eq!(reach(&ordered, 130.0), Ok(true));
    let defend = Seeker {
        stance: 1,
        ..STAND_GROUND
    };
    assert_eq!(reach(&defend, 124.2), Ok(true));
    assert_eq!(reach(&defend, 124.3), Ok(false));
    // never in NoAttack, never an object its targeting flag leaves out, never class 1
    let no_attack = Seeker {
        stance: 3,
        ..STAND_GROUND
    };
    assert_eq!(reach(&no_attack, 119.0), Ok(false));
    let untargeted = Target {
        targeting: false,
        ..villager(119.0)
    };
    assert_eq!(
        soldier_can_attack(&SCOUT, &STAND_GROUND, &untargeted),
        Ok(false)
    );
    let relic = Target {
        class: 1,
        ..villager(119.0)
    };
    assert_eq!(soldier_can_attack(&SCOUT, &STAND_GROUND, &relic), Ok(false));
}

#[test]
fn an_idle_tower_searches_with_its_classes() {
    let mut u = UnitAi {
        idle_timer: 504,
        adjusted_idle_timeout: 503,
        ..idle(15)
    };
    let at = Surroundings {
        position: [29.0, 94.0, 0.0],
        player: 2,
        size: [2.0, 2.0],
        los: 8.0,
        search_radius: 0.0,
        state: 2,
        kind: 80,
        ..Surroundings::default()
    };
    let mut searched = Vec::new();
    let mut closest = |s: &Search<'_>| {
        searched.push((s.radius, s.classes.to_vec()));
        Ok(Found::NONE)
    };
    let fx = u
        .update(28, 0, false, &at, &mut rng(), &mut closest)
        .unwrap();
    assert_eq!(fx, vec![Effect::CapHumanOrder(5000)]);
    assert_eq!(searched, vec![(9.0, TOWER_CARED.to_vec())]);
}

/// `FUN_140c4dcb0`: both targets and their classes go and the point is −1 on all three axes;
/// the order and action stay. MOVE's notify to a unit that took its `MoveTo` forgets the same
/// way before it takes the order's target.
#[test]
fn forgetting_the_targets_clears_the_current_and_the_last_and_the_point() {
    let mut u = UnitAi {
        current_order: ATTACK_ORDER,
        current_action: ATTACK_ACTION,
        current_target: 12,
        target_type: 4,
        target_point: [3.0, 4.0, 0.5],
        last_target: 11,
        last_target_type: 9,
        ..idle(13)
    };
    u.forget_targets();
    assert_eq!(
        (
            u.current_target,
            u.target_type,
            u.last_target,
            u.last_target_type
        ),
        (-1, -1, -1, -1)
    );
    assert_eq!(u.target_point, [-1.0; 3]);
    assert_eq!(
        (u.current_order, u.current_action),
        (ATTACK_ORDER, ATTACK_ACTION)
    );
    let mut moved = UnitAi {
        current_target: 12,
        target_type: 4,
        last_target: 11,
        last_target_type: 9,
        ..idle(13)
    };
    moved.notify_move(true, None, [5.0, 6.0], &mut rng());
    // the forget, then FUN_140c64be0 with no target: the forgotten -1 becomes the last
    assert_eq!(
        (
            moved.last_target,
            moved.last_target_type,
            moved.current_target
        ),
        (-1, -1, -1)
    );
    assert_eq!(moved.target_point, [5.0, 6.0, 0.0]);
}

/// `FUN_140c4eea0`: a master with a task of action type 0x99 refuses orders while its target
/// exists or it has an action value; with the second argument set a unit that passes takes
/// orders only with that task, and without it any unit that passes does.
#[test]
fn a_unit_takes_orders_by_its_0x99_task_and_with_task_only_needs_it() {
    let mut u = idle(13);
    let plain = Surroundings::default();
    let carter = Surroundings {
        task_0x99: true,
        ..Surroundings::default()
    };
    assert!(u.can_order(&plain, false));
    assert!(!u.can_order(&plain, true));
    assert!(u.can_order(&carter, false));
    assert!(u.can_order(&carter, true));
    u.current_action = GATHER_ACTION;
    assert!(!u.can_order(&carter, false));
    assert!(!u.can_order(&carter, true));
    assert!(u.can_order(&plain, false));
    u.current_action = -1;
    let targeted = Surroundings {
        target: Some(Seen::default()),
        ..carter
    };
    assert!(!u.can_order(&targeted, true));
}

/// `FUN_140c1a760` at `LAB_140c1aa59`: an enemy building (master type `'P'`) passes the
/// tower's check once a builder started it; a foundation still desolid (`+0xc60`) does not.
/// The flag counts only for a building: a unit that holds it passes.
#[test]
fn a_tower_attacks_a_started_building_but_not_a_desolid_foundation() {
    let house = Target {
        owner: 1,
        class: 3,
        kind: 80,
        master: 70,
        ..BOAR
    };
    assert_eq!(tower_can_attack(&TC, &house), Ok(true));
    let foundation = Target {
        desolid: true,
        ..house
    };
    assert_eq!(tower_can_attack(&TC, &foundation), Ok(false));
    let unit = Target {
        owner: 1,
        class: 6,
        desolid: true,
        ..BOAR
    };
    assert_eq!(tower_can_attack(&TC, &unit), Ok(true));
}

/// `FUN_140b43f30` on the attackers (`+0xc0`): an id joins the set in its sorted place and
/// the call answers 1; an id already in it changes nothing and answers 0.
#[test]
fn an_attacker_joins_the_sorted_set_once() {
    let mut u = idle(4);
    assert!(u.add_attacker(30));
    assert!(u.add_attacker(10));
    assert!(u.add_attacker(20));
    assert!(!u.add_attacker(10));
    assert!(u.add_attacker(-1));
    assert_eq!(u.attackers, vec![-1, 10, 20, 30]);
}

/// The soldier's slot 0xa8 (`FUN_140c1ae60`): a neutral object of a class in the mask
/// `0x200200800838001c` is passed over; at attack priority 3 only [`PRIORITY_3_CLASSES`]
/// pass; any other class passes, also one outside the mask's 62 bits.
#[test]
fn the_soldier_passes_over_neutrals_of_its_masked_classes() {
    for class in [2, 3, 4, 19, 20, 21, 27, 39, 49, 61] {
        assert!(!soldier_attacks_neutral(0, class), "class {class}");
    }
    for class in [0, 6, 9, 15, 35, 52, 53, 60, 62, 63, -1] {
        assert!(soldier_attacks_neutral(0, class), "class {class}");
    }
    assert!(soldier_attacks_neutral(3, 52));
    assert!(!soldier_attacks_neutral(3, 6));
    // a masked class stays passed over whatever the priority
    assert!(!soldier_attacks_neutral(3, 27));
}

/// `FUN_140c42640`: an order goes after the last unless it goes in front of a queue that
/// holds one.
#[test]
fn an_inserted_order_goes_in_front_only_of_a_queue_that_holds_one() {
    let order = |target| QueuedOrder {
        target,
        order: MOVE_ORDER,
        ready: true,
        ..QueuedOrder::default()
    };
    let mut u = idle(13);
    u.insert_order(order(1), true);
    u.insert_order(order(2), false);
    u.insert_order(order(3), true);
    u.insert_order(order(4), false);
    let targets: Vec<i32> = u.orders.iter().map(|o| o.target).collect();
    assert_eq!(targets, [3, 1, 2, 4]);
}

/// `FUN_140c55c90` immediate and in front: a builder (702/602 on foundation 50) told to
/// move at once keeps its build order: the build goes in front of the queue with the move's
/// shown point and ready flag (issuer the unit, owner −1, its range `+0x5c`), the move in
/// front of it; the target goes, the action stops and ends the build order (602 + 100), the
/// move runs (slot 0x230) and leaves the queue; the build waits behind.
#[test]
fn an_immediate_order_in_front_keeps_the_current_order_behind_it() {
    let mut u = UnitAi {
        current_order: BUILD_ORDER,
        order_priority: 2,
        current_action: BUILD_ACTION,
        current_target: 50,
        target_type: 3,
        target_point: [20.0, 21.0, 0.0],
        desired_distance: 2.0,
        ..idle(8)
    };
    let order = QueuedOrder {
        issuer: 1,
        order: MOVE_ORDER,
        priority: 7,
        target: -1,
        target_owner: -1,
        point: [10.0, 11.0, 0.0],
        range: 0.5,
        shown: [10.0, 11.0],
        ready: true,
        target_type: -1,
    };
    let mut fx = Vec::new();
    u.order_now(
        order,
        Some(9),
        Some(([5.0, 6.0], 4)),
        None,
        &mut rng(),
        &mut fx,
    )
    .unwrap();
    let kept = QueuedOrder {
        issuer: 9,
        order: BUILD_ORDER,
        priority: 2,
        target: 50,
        target_owner: -1,
        point: [20.0, 21.0, 0.0],
        range: 2.0,
        shown: [10.0, 11.0],
        ready: true,
        target_type: 3,
    };
    assert_eq!(u.orders, vec![kept]);
    assert_eq!(
        fx,
        vec![
            Effect::Stop,
            Effect::Move(AiMove {
                point: [10.0, 11.0, 0.0],
                range: 0.5,
                force: true,
                walk: true,
            })
        ]
    );
    assert_eq!((u.current_order, u.order_priority), (MOVE_ORDER, 7));
    // the stop ended the build order; the move's FUN_140c64c10 made the forgotten -1 the
    // last target
    assert_eq!((u.last_order, u.last_target), (BUILD_ORDER, -1));
    assert_eq!(u.state_position, [5.0, 6.0]);
}

/// `FUN_140c55c90` immediate for an attack roundup (0x2d9): "We have an `AttackRoundup`, so
/// I'm not stopping the object.": the target stays and nothing stops; the notifies still go
/// and slot 0x230 runs the order in front: its roundup (slot 0xc0) attacks the new target,
/// and the tail makes the order 700. Outside the update (no surroundings) the case does not
/// run.
#[test]
fn an_immediate_attack_roundup_keeps_the_target_until_it_attacks_the_next() {
    let attacking = || UnitAi {
        current_order: ATTACK_ORDER,
        current_action: ATTACK_ACTION,
        current_target: 50,
        notifies: vec![Notify::own(9, DAMAGED, [1, 5, 0])],
        ..idle(13)
    };
    let roundup = QueuedOrder {
        order: ai_kinds::ORDER_ATTACK_ROUNDUP,
        priority: 100,
        target: 51,
        ready: true,
        ..QueuedOrder::default()
    };
    let mut u = attacking();
    let mut fx = Vec::new();
    let r = u.take_order_now(2, roundup, ([0.0; 2], 6), &mut rng(), &mut fx);
    assert!(r.is_err());
    assert_eq!(fx, Vec::new());
    assert_eq!((u.current_target, u.current_action), (50, ATTACK_ACTION));
    assert_eq!(u.notifies, Vec::new());
    let mut u = attacking();
    let mut side = OrderSide::holding(&[(51, 6)]);
    side.reach = true;
    let at = Surroundings {
        class: 6,
        ..Surroundings::default()
    };
    let mut fx = Vec::new();
    u.order_now(
        roundup,
        None,
        None,
        Some((&at, &mut side, false)),
        &mut rng(),
        &mut fx,
    )
    .unwrap();
    assert_eq!(fx, Vec::new());
    assert_eq!(side.attacks, [(51, true)]);
    assert_eq!(
        (
            u.current_target,
            u.last_target,
            u.current_action,
            u.last_action
        ),
        (51, 50, ATTACK_ACTION, ATTACK_ACTION)
    );
    assert_eq!((u.current_order, u.order_priority), (ATTACK_ORDER, 100));
    assert_eq!((u.notifies.len(), u.orders.len()), (0, 0));
}

/// `FUN_140c5e7d0`: a unit more than a tile (the squared footprint gap, `FUN_140a847c0`)
/// from its state position moves there at its height within 1.0, forced and running (the
/// unit's slot 0x340); the move action value becomes current, the old one the last, and the
/// target stays. Within a tile nothing happens. A gap that is not a number walks too
/// (`COMISS 1.0, gap; JNC`).
#[test]
fn a_unit_off_its_state_position_walks_back_and_keeps_its_target() {
    let mut u = UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 77,
        state_position: [10.0, 10.0],
        ..idle(13)
    };
    let at = |x: f32| Surroundings {
        position: [x, 10.0, 2.5],
        size: [0.5, 0.5],
        ..Surroundings::default()
    };
    let mut fx = Vec::new();
    // 1.4 off, 0.9 off its footprint: within a tile
    assert!(!u.walk_back(&at(11.4), &mut fx));
    assert_eq!((fx.len(), u.current_action), (0, ATTACK_ACTION));
    assert!(u.walk_back(&at(11.6), &mut fx));
    assert_eq!(
        fx,
        vec![Effect::Move(AiMove {
            point: [10.0, 10.0, 2.5],
            range: 1.0,
            force: true,
            walk: false,
        })]
    );
    assert_eq!(
        (u.last_action, u.current_action),
        (ATTACK_ACTION, MOVE_ACTION)
    );
    assert_eq!(u.current_target, 77);
    assert!(u.commanded);
    let mut lost = UnitAi {
        state_position: [f32::NAN, 10.0],
        ..idle(13)
    };
    assert!(lost.walk_back(&at(10.0), &mut Vec::new()));
}

/// `FUN_140a847c0` clamps each axis by `MAXSS 0, gap`: a gap that is not a number stays one.
#[test]
fn the_point_gap_keeps_a_gap_that_is_not_a_number() {
    assert_eq!(
        point_gap_squared([3.0, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0]),
        4.0
    );
    assert!(point_gap_squared([f32::NAN, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0]).is_nan());
}

/// `FUN_140c20db0` in `NoAttack` ("searching and returning to state pos."): the search alone
/// (slot 0x50 with mode 1), then off the patrol orders the walk back; processIdle stays 5.
#[test]
fn a_no_attack_soldier_searches_then_walks_back_to_its_state_position() {
    let (mut u, at) = idle_scout();
    u.stance = 3;
    u.state_position = [25.0, 92.5];
    let mut picks = Vec::new();
    let mut look = |s: &Search<'_>| {
        picks.push(s.pick);
        Ok(Found::NONE)
    };
    let fx = u.update(28, 0, false, &at, &mut rng(), &mut look).unwrap();
    assert_eq!(picks, vec![Pick::Look]);
    assert_eq!(
        fx,
        vec![
            Effect::CapHumanOrder(5000),
            Effect::Move(AiMove {
                point: [25.0, 92.5, 0.0],
                range: 1.0,
                force: true,
                walk: false,
            })
        ]
    );
    // on a patrol order it stays
    let (mut u, at) = idle_scout();
    u.stance = 3;
    u.state_position = [25.0, 92.5];
    u.current_order = PATROL;
    let mut fx = Vec::new();
    u.no_attack_idle(&at, false, &mut |_: &Search<'_>| Ok(Found::NONE), &mut fx)
        .unwrap();
    assert_eq!(fx, Vec::new());
}

/// The full update (`0x140c66b10`) of a unit in a group the world holds: its state position
/// follows it while its group has a command (`+0x3a8`); its idle check (slot 0x240) waits
/// while its group is not idle (`FUN_141017ad0`) or the unit moves (slot 0x5c8); its order
/// queue waits while the group is not idle; a group that patrols takes the patrol's idle
/// part, and its target's range test (`FUN_140c41690`) answers 0 for a human player; action
/// `0x2c0` (moved aside) ends, the last action keeping it, unless the group moves.
#[test]
fn a_grouped_unit_s_full_update_reads_its_group() {
    let group = |idle, moving, member_moving| {
        Some(GroupState {
            commanded: true,
            idle,
            patrol: false,
            moving,
            member_moving,
        })
    };
    let ship = || UnitAi {
        idle_timer: 3001,
        adjusted_idle_timeout: 3000,
        ..idle(11)
    };
    let at = |group| Surroundings {
        position: [3.0, 4.0, 0.0],
        player: 1,
        class: 20,
        group,
        ..Surroundings::default()
    };
    let run =
        |u: &mut UnitAi, at: &Surroundings| u.update(28, 0, false, at, &mut rng(), &mut no_search);
    // an idle group of a still unit: the idle check runs, the state position follows
    let mut u = ship();
    let fx = run(&mut u, &at(group(true, false, false))).unwrap();
    assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
    assert_eq!(u.state_position, [3.0, 4.0]);
    // a group not idle, or a moving unit: no idle check
    for g in [group(false, true, false), group(true, false, true)] {
        let mut u = ship();
        assert_eq!(run(&mut u, &at(g)), Ok(vec![]));
        assert_eq!(u.idle_timer, 3029);
    }
    // a group without a command: the state position stays
    let mut u = ship();
    let mut g = group(true, false, false);
    g.as_mut().unwrap().commanded = false;
    let _ = run(&mut u, &at(g)).unwrap();
    assert_eq!(u.state_position, [0.0; 2]);
    // the order queue waits for a group that is not idle
    let order = QueuedOrder {
        issuer: 1,
        order: MOVE_ORDER,
        priority: 100,
        target: -1,
        target_owner: -1,
        point: [9.0, 9.0, 0.0],
        range: 0.0,
        shown: [9.0, 9.0],
        ready: true,
        target_type: -1,
    };
    let mut u = UnitAi {
        orders: vec![order],
        ..idle(11)
    };
    let _ = run(&mut u, &at(group(false, true, false))).unwrap();
    assert_eq!(u.orders.len(), 1);
    let _ = run(&mut u, &at(group(true, false, false))).unwrap();
    assert_eq!(u.orders.len(), 0);
    // a patrolling group: the patrol branch, its secondary timer run out, restarts it at 750
    // and runs processIdle (the ship's caps its last human order time)
    let mut u = idle(11);
    let mut g = group(false, true, false);
    g.as_mut().unwrap().patrol = true;
    assert_eq!(
        run(&mut u, &at(g)),
        Ok(vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)])
    );
    assert_eq!(u.secondary_timer, 750);
    // a target of a unit in a patrolling group: FUN_140c41690 answers 0 for a human player
    let mut g = group(true, false, false);
    g.as_mut().unwrap().patrol = true;
    let seen = Seen {
        state: 2,
        kind: 70,
        owner: 2,
        class: 6,
        position: [5.0, 5.0, 0.0],
        radius: [0.2, 0.2],
        visible: true,
        flags: 0,
        sight: 0xf,
    };
    let target_at = Surroundings {
        target: Some(seen),
        state: 2,
        ..at(g)
    };
    let mut u = UnitAi {
        current_target: 5,
        ..idle(11)
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.target_block(&target_at, false, &mut rng(), &mut no_search, &mut fx),
        Ok(())
    );
    assert_eq!(
        u.target_block(&target_at, true, &mut rng(), &mut no_search, &mut fx),
        Err(GenieError::unsupported(
            "the patrol range of a unit AI's target"
        ))
    );
    // action 0x2c0 ends unless the group moves
    for (g, after) in [
        (None, -1),
        (group(true, false, false), -1),
        (group(false, true, false), GROUP_ASIDE_ACTION),
    ] {
        let mut u = UnitAi {
            current_action: GROUP_ASIDE_ACTION,
            last_action: 7,
            ..idle(11)
        };
        let _ = run(&mut u, &at(g)).unwrap();
        assert_eq!(u.current_action, after);
        let last = if after == -1 { GROUP_ASIDE_ACTION } else { 7 };
        assert_eq!(u.last_action, last);
    }
}

/// The transport ship's processIdle (`0x140c21d20`): `FUN_140c41e50`, then 5. An idle ship
/// whose idle timer passed its timeout caps its last human order time and does nothing more.
#[test]
fn an_idle_transport_ship_s_check_caps_its_human_order_time_and_does_nothing() {
    let mut u = UnitAi {
        idle_timer: 3001,
        adjusted_idle_timeout: 3000,
        ..idle(11)
    };
    let at = Surroundings {
        player: 1,
        class: 20,
        ..Surroundings::default()
    };
    let fx = u
        .update(28, 0, false, &at, &mut rng(), &mut no_search)
        .unwrap();
    assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
    assert_eq!(u.idle_timer, 0);
    // the building's (FUN_140c1e600) and the livestock's (FUN_140c1fea0) return 5 alone
    for ai_type in [17, 19] {
        let mut u = idle(ai_type);
        let mut fx = Vec::new();
        assert_eq!(
            u.process_idle(&at, false, &mut rng(), &mut no_search, &mut fx),
            Ok(false)
        );
        assert_eq!(fx, Vec::new());
    }
}

/// The target block's attack whose target died, for a villager: the civilian's processIdle
/// (`FUN_140c1e610`: the cap, then 5) produces nothing, so the AI moves to the dead target's
/// point within its weapon range and the order ends.
#[test]
fn a_villager_whose_attack_target_died_walks_to_its_point() {
    let mut u = UnitAi {
        current_order: ATTACK_ORDER,
        order_priority: 100,
        current_action: ATTACK_ACTION,
        current_target: 50,
        target_point: [5.0, 6.0, 0.0],
        ..idle(8)
    };
    let at = Surroundings {
        weapon_range: 0.25,
        ..Surroundings::default()
    };
    let mut fx = Vec::new();
    u.next_after_kill(&at, false, &mut rng(), &mut no_search, &mut fx)
        .unwrap();
    assert_eq!(
        fx,
        vec![
            Effect::CapHumanOrder(HUMAN_ORDER_CAP),
            Effect::Move(AiMove {
                point: [5.0, 6.0, 0.0],
                range: 0.25,
                force: true,
                walk: false,
            })
        ]
    );
    assert_eq!(
        (u.current_order, u.order_priority, u.last_order),
        (-1, -1, ATTACK_ORDER)
    );
}

/// A wolf whose attack target died: its processIdle (`FUN_140c1fb70`) finds another villager
/// within its reach and calls `AttackObject` unforced, which fails while the attack action
/// value is still current; the 6 it gives all the same keeps it from walking to the dead
/// one's point.
#[test]
fn a_wolf_whose_attack_target_died_attacks_the_next_villager() {
    let mut u = UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 50,
        target_point: [5.0, 6.0, 0.0],
        ..idle(5)
    };
    let at = Surroundings {
        los: 4.0,
        search_radius: 1.0,
        ..Surroundings::default()
    };
    let mut side = LionSide {
        found: [700, -1],
        searches: 0,
        seen: UnitSeen {
            id: 700,
            class: 4,
            owner: 1,
            state: 2,
            gap_squared: 1.0,
            ..UnitSeen::default()
        },
        attacks: Vec::new(),
    };
    let mut fx = Vec::new();
    u.next_after_kill(&at, false, &mut rng(), &mut side, &mut fx)
        .unwrap();
    assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
    assert_eq!(side.attacks, Vec::new());
    assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 50));
}

/// The tower in `NoAttack` (stance 3, `FUN_140c564c0`): after the cap only the search (slot
/// 0x50 with mode 1, its cared classes), whose alert mode it sets; then 5.
#[test]
fn a_no_attack_tower_only_searches() {
    let mut u = UnitAi {
        idle_timer: 504,
        adjusted_idle_timeout: 503,
        stance: 3,
        ..idle(15)
    };
    let at = Surroundings {
        position: [29.0, 94.0, 0.0],
        player: 2,
        size: [2.0, 2.0],
        los: 8.0,
        state: 2,
        kind: 80,
        ..Surroundings::default()
    };
    let mut searched = Vec::new();
    let mut closest = |s: &Search<'_>| {
        searched.push((s.radius, s.classes.len(), s.pick));
        Ok(Found {
            id: -1,
            enemies: true,
            position: None,
            fresh: true,
            players: 0,
            distance: -1,
            retries: [None; 3],
        })
    };
    let fx = u
        .update(28, 0, false, &at, &mut rng(), &mut closest)
        .unwrap();
    assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
    assert_eq!(searched, vec![(9.0, TOWER_CARED.len(), Pick::Look)]);
    assert!(u.alert_mode);
}

/// The tower's processIdle in the idle part (`FUN_140c564c0` with 1): what
/// `bestUnitToAttack` found it attacks (slot 0xb8 forced); the idle part reads no result.
#[test]
fn an_idle_tower_attacks_what_its_search_found() {
    let mut u = UnitAi {
        idle_timer: 504,
        adjusted_idle_timeout: 503,
        ..idle(15)
    };
    let at = Surroundings {
        position: [29.0, 94.0, 0.0],
        player: 2,
        size: [2.0, 2.0],
        los: 8.0,
        state: 2,
        kind: 80,
        ..Surroundings::default()
    };
    let mut side = SoldierSide {
        found: 9731,
        attacks: Vec::new(),
        patrol: None,
        refuses: false,
    };
    let fx = u.update(28, 0, false, &at, &mut rng(), &mut side).unwrap();
    assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
    assert_eq!(side.attacks, vec![(9731, true)]);
    assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 9731));
}

/// The hunted animal's processIdle for a king (class 59, `FUN_140c1eea0`): after the cap a
/// human player's king does nothing (5) and draws nothing; only a computer player's (not of
/// master 704, not inside another, without an order) has its unit's slot 0x7e8, which is not
/// modelled.
#[test]
fn a_human_player_s_king_s_idle_check_only_caps() {
    let at = Surroundings {
        position: [31.5, 106.5, 0.0],
        player: 1,
        class: CLASS_KING,
        master: 434,
        ..Surroundings::default()
    };
    let mut u = UnitAi {
        hunted: None,
        ..deer()
    };
    let mut r = rng();
    let mut fx = Vec::new();
    assert_eq!(
        u.process_idle(&at, false, &mut r, &mut no_search, &mut fx),
        Ok(false)
    );
    assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
    assert_eq!(r.draws, 0);
    assert!(
        u.process_idle(&at, true, &mut r, &mut no_search, &mut Vec::new())
            .is_err()
    );
    // a computer player's king of master 704, or one with an order, does nothing either
    let unled = Surroundings {
        master: KING_UNLED,
        ..at.clone()
    };
    assert_eq!(
        u.process_idle(&unled, true, &mut r, &mut no_search, &mut Vec::new()),
        Ok(false)
    );
    u.current_order = MOVE_ORDER;
    assert_eq!(
        u.process_idle(&at, true, &mut r, &mut no_search, &mut Vec::new()),
        Ok(false)
    );
}

/// The civilian's `AttackObject` (slot 0xb8, `FUN_140c17370`, forced): an object the world
/// does not hold gets nothing; an animal or a tree (classes 9, 10, 15) is hunted (slot 0xc8);
/// any other goes to the base `AttackObject`, whose civilian slot 0xa0 (`FUN_140c18b70`)
/// refuses classes 1 and 42, and at attack priority 3 the classes outside
/// [`PRIORITY_3_CLASSES`].
#[test]
fn a_villager_hunts_an_animal_and_attacks_another_object() {
    let at = |priority: u8| Surroundings {
        id: 10041,
        state: 2,
        attack_priority: priority,
        units: vec![
            attacker(9731, CLASS_10, 0.5),
            attacker(9800, 6, 1.0),
            attacker(9801, 42, 1.0),
            attacker(9802, 3, 1.0),
        ],
        ..Surroundings::default()
    };
    let mut u = idle(8);
    let mut side = Recorder::default();
    let mut fx = Vec::new();
    let mut nobody = LionSide {
        found: [-1, -1],
        searches: 0,
        seen: UnitSeen::default(),
        attacks: Vec::new(),
    };
    assert_eq!(
        u.civilian_attack(9999, &at(0), &mut nobody, &mut fx),
        Ok(false)
    );
    assert_eq!(
        u.civilian_attack(9731, &at(0), &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(fx, vec![Effect::HuntObject(9731)]);
    assert_eq!((u.current_action, u.current_target), (HUNT_ACTION, 9731));
    let mut u = idle(8);
    let mut fx = Vec::new();
    assert_eq!(
        u.civilian_attack(9800, &at(0), &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(side.attacks, vec![(9800, true)]);
    assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 9800));
    let mut u = idle(8);
    assert_eq!(
        u.civilian_attack(9801, &at(0), &mut side, &mut fx),
        Ok(false)
    );
    assert_eq!(
        u.civilian_attack(9800, &at(3), &mut side, &mut fx),
        Ok(false)
    );
    assert_eq!(
        u.civilian_attack(9802, &at(3), &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(side.attacks, vec![(9800, true), (9802, true)]);
}

/// Slot 0xa0 of the types whose `AttackObject` is the base's: the transport ship's and the
/// livestock's (`FUN_140c48400`) pass anything, the building's (`FUN_140c18b60`) nothing.
#[test]
fn a_ship_or_livestock_may_attack_anything_and_a_building_nothing() {
    let at = Surroundings {
        id: 10041,
        state: 2,
        units: vec![attacker(9800, 6, 1.0)],
        ..Surroundings::default()
    };
    for (ai_type, attacks) in [(11, true), (19, true), (17, false)] {
        let mut u = idle(ai_type);
        let mut side = Recorder::default();
        let r = u.attack_object(9800, true, false, &at, &mut side, &mut Vec::new());
        assert_eq!(r, Ok(attacks), "type {ai_type}");
    }
}

/// A notify through the type's handler (slot 0x238) with a recording side and no world.
fn handled(
    u: &mut UnitAi,
    n: Notify,
    at: &Surroundings,
    computer: bool,
) -> (Result<u32, GenieError>, Vec<Effect>, Recorder) {
    let mut side = Recorder::default();
    let mut fx = Vec::new();
    let r = u.handle(n, 0, at, computer, &mut rng(), &mut side, &mut fx);
    (r, fx, side)
}

/// The boar's handler (`FUN_140c25770`) for the completion of its move off a footprint
/// (0x1fa on action 0x2bf): its idle timeout becomes the adjusted one plus 1, the target goes
/// (`FUN_140c5e1e0`), its slot 0x1f0 (`FUN_140c16d50`) sends it home on its next idle check,
/// then the base handler stops the completed action.
#[test]
fn a_boar_whose_move_aside_completed_heads_home_next() {
    let mut u = UnitAi {
        current_action: MOVE_ASIDE_ACTION,
        current_target: 50,
        target_type: 4,
        adjusted_idle_timeout: 3100,
        idle_timeout: 3000,
        ..hit_boar()
    };
    let n = Notify::own(9729, ACTION_COMPLETED, [MOVE_ASIDE_ACTION, 0, 0]);
    let (r, fx, _) = handled(&mut u, n, &Surroundings::default(), false);
    assert_eq!(r, Ok(GO_ON));
    assert_eq!(fx, vec![Effect::Stop]);
    assert_eq!(u.idle_timeout, 3101);
    assert_eq!((u.current_target, u.last_target), (-1, 50));
    assert_eq!(u.current_action, -1);
    assert!(u.home.is_some_and(|h| h.go_home));
}

/// The lion's handler (`FUN_140c266f0`): a completed attack (0x1fa with 600) sets its idle
/// timeout to 1000 and redraws the adjusted one (`FUN_140c41bc0`), drops the target and
/// stops (slot 0xb0 with 1); the base handler then stops on the attack's completion again.
#[test]
fn a_lion_s_completed_attack_resets_its_idle_timeout_and_stops() {
    let mut u = UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 700,
        idle_timeout: 6000,
        ..idle(5)
    };
    let n = Notify::own(800, ACTION_COMPLETED, [ATTACK_ACTION, 0, 0]);
    let mut r = rng();
    let expected = adjusted_idle_timeout(1000, &mut rng());
    let mut fx = Vec::new();
    let result = u.handle(
        n,
        0,
        &Surroundings::default(),
        false,
        &mut r,
        &mut Recorder::default(),
        &mut fx,
    );
    assert_eq!(result, Ok(GO_ON));
    assert_eq!((u.idle_timeout, u.adjusted_idle_timeout), (1000, expected));
    assert_eq!(r.draws, 1);
    assert_eq!(fx, vec![Effect::Stop, Effect::Stop]);
    assert_eq!(
        (u.current_target, u.last_target, u.current_action),
        (-1, 700, -1)
    );
}

/// The lion's handler for a moved target (0x1fc): a target the world no longer holds, or
/// one beyond its line of sight (the footprints' squared gap against its sight squared,
/// `COMISS` then `JC`), or one whose master has flag 0x10 (`+0x654`), is dropped and the lion
/// stops before the base handler; one within its sight it attacks again (slot 0xb8 with the
/// target, forced) and the processing breaks (3).
#[test]
fn a_lion_drops_a_moved_target_out_of_its_sight() {
    let lion = || UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 700,
        ..idle(5)
    };
    let n = Notify::own(800, TARGET_MOVED, [0, 0, 0]);
    let mut u = lion();
    let (r, fx, _) = handled(&mut u, n, &Surroundings::default(), false);
    assert_eq!((r, fx), (Ok(GO_ON), vec![Effect::Stop]));
    assert_eq!((u.current_target, u.current_action), (-1, -1));
    // the villager 700 the lion attacks (class 4, which it attacks by its classes)
    let at = |x: f32| Surroundings {
        position: [10.0, 10.0, 0.0],
        size: [0.5, 0.5],
        los: 4.0,
        target: Some(Seen {
            state: 2,
            class: 4,
            position: [x, 10.0, 0.0],
            radius: [0.5, 0.5],
            ..Seen::default()
        }),
        units: vec![UnitSeen {
            id: 700,
            class: 4,
            owner: 1,
            state: 2,
            position: [x, 10.0, 0.0],
            ..UnitSeen::default()
        }],
        ..Surroundings::default()
    };
    // 5 tiles apart, 4 between the footprints: at its sight, attacked again
    let mut u = lion();
    let (r, fx, side) = handled(&mut u, n, &at(15.0), false);
    assert_eq!(
        (r, fx, side.attacks),
        (Ok(BREAK), vec![], vec![(700, true)])
    );
    assert_eq!((u.current_target, u.current_action), (700, ATTACK_ACTION));
    // a master with flag 0x10 is dropped at its sight too
    let mut u = lion();
    let mut flagged = at(15.0);
    flagged.target = flagged.target.map(|t| Seen { flags: 0x10, ..t });
    let (r, fx, side) = handled(&mut u, n, &flagged, false);
    assert_eq!(
        (r, fx, side.attacks),
        (Ok(GO_ON), vec![Effect::Stop], vec![])
    );
    // 6 apart, 5 between: beyond it
    let mut u = lion();
    let (r, fx, _) = handled(&mut u, n, &at(16.0), false);
    assert_eq!((r, fx), (Ok(GO_ON), vec![Effect::Stop]));
    assert_eq!(u.current_target, -1);
}

/// The building's handler (`FUN_140c22610`): an attack (600) adds its attacker and puts it
/// under attack; 699 drops one; damage (500) at no hit points ends the notifies (4), else
/// adds an attacker already known or its target; a new one makes a human player's building
/// ask for help (`FUN_140c42ce0`, not modelled), and a computer player's AI hears of it.
#[test]
fn a_building_counts_its_attackers() {
    let mut u = idle(17);
    let at = Surroundings::default();
    let (r, _, side) = handled(&mut u, Notify::own(70, 600, [0; 3]), &at, false);
    assert_eq!((r, side.under_attack), (Ok(GO_ON), Some(true)));
    let _ = handled(&mut u, Notify::own(60, 600, [0; 3]), &at, false);
    assert_eq!(u.attackers, vec![60, 70]);
    let (r, _, _) = handled(&mut u, Notify::own(70, NO_LONGER_ON_ME, [0; 3]), &at, false);
    assert_eq!((r, u.attackers.clone()), (Ok(GO_ON), vec![60]));
    let (r, _, _) = handled(&mut u, Notify::own(99, NO_LONGER_ON_ME, [0; 3]), &at, false);
    assert_eq!((r, u.attackers.clone()), (Ok(GO_ON), vec![60]));
    let hit = |caller, hp| Notify::own(caller, DAMAGED, [caller, hp, 0]);
    let (r, _, side) = handled(&mut u, hit(60, 5), &at, false);
    assert_eq!((r, side.under_attack), (Ok(GO_ON), Some(true)));
    u.current_target = 61;
    let (r, _, _) = handled(&mut u, hit(61, 5), &at, false);
    assert_eq!((r, u.attackers.clone()), (Ok(GO_ON), vec![60, 61]));
    assert!(handled(&mut u, hit(62, 5), &at, false).0.is_err());
    assert!(handled(&mut u, hit(60, 5), &at, true).0.is_err());
    u.notifies = vec![hit(60, 0)];
    let (r, _, _) = handled(&mut u, hit(60, 0), &at, false);
    assert_eq!((r, u.notifies.len()), (Ok(DEAD), 0));
}

/// The tower's handler (`FUN_140c299f0`) for damage at no hit points: the owner's alert
/// (0x209, a human player's alert list), the notifies end and it gives up (4).
#[test]
fn a_destroyed_tower_gives_up() {
    let mut u = UnitAi {
        notifies: vec![Notify::own(60, DAMAGED, [60, 0, 0])],
        ..idle(15)
    };
    let n = Notify::own(60, DAMAGED, [60, 0, 0]);
    let (r, _, _) = handled(&mut u, n, &Surroundings::default(), false);
    assert_eq!((r, u.notifies.len()), (Ok(DEAD), 0));
    let mut u = idle(15);
    assert!(
        handled(&mut u, n, &Surroundings::default(), true)
            .0
            .is_err()
    );
}

/// The hunted animal's evade goes through `evasiveMoveTo` (slot 0x150 with 1 and 0,
/// `FUN_140c4d140`): an object on its own point gives a point that is not a number, which
/// becomes its own place minus one on each axis (above 5) before the map's hold.
#[test]
fn a_deer_evading_an_object_on_its_own_point_steps_off_by_one() {
    let mut u = deer();
    let mut r = wander_seed(false);
    let place = Surroundings {
        speed: 1.2,
        map: [120, 120],
        ..deer_at([31.5, 106.5, 0.0])
    };
    let on_top = |_: &Search<'_>| {
        Ok(Found {
            id: 9183,
            enemies: false,
            position: Some([31.5, 106.5, 0.0]),
            fresh: true,
            players: 0,
            distance: -1,
            retries: [None; 3],
        })
    };
    let moved = u
        .update_idle(10, 0, false, &place, &mut r, &mut on_top.clone())
        .unwrap()
        .unwrap();
    assert_eq!(moved.point, [30.5, 105.5, 0.0]);
    let near_edge = Surroundings {
        speed: 1.2,
        map: [120, 120],
        ..deer_at([3.5, 4.5, 0.0])
    };
    let mut u = deer();
    let on_it = |_: &Search<'_>| {
        Ok(Found {
            id: 9183,
            enemies: false,
            position: Some([3.5, 4.5, 0.0]),
            fresh: true,
            players: 0,
            distance: -1,
            retries: [None; 3],
        })
    };
    let moved = u
        .update_idle(
            10,
            0,
            false,
            &near_edge,
            &mut wander_seed(false),
            &mut on_it.clone(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(moved.point, [4.5, 5.5, 0.0]);
}

/// A unit side whose stand test fails a given number of times.
struct StandSide {
    refusals: usize,
    tested: Vec<[f32; 2]>,
}

impl UnitSide for StandSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Err(GenieError::unsupported("no attack here"))
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn can_stand(&mut self, point: [f32; 2]) -> Result<bool, GenieError> {
        self.tested.push(point);
        if self.refusals == 0 {
            return Ok(true);
        }
        self.refusals -= 1;
        Ok(false)
    }
}

/// `evasiveMoveTo` (`FUN_140c4d140`): without `uncon` a unit with an action stays; with
/// `tries` a point the unit cannot stand at moves by four draws (x's sign, its amount, y's
/// sign, its amount: `rand15 & 1`, `rand15 & 3`), held on the map, up to five times; then the
/// forced running move within 0, the move action value and the point, the AI commanded.
#[test]
fn an_evasive_move_steps_off_a_point_the_unit_cannot_stand_at() {
    let at = Surroundings {
        position: [10.0, 20.0, 1.5],
        map: [64, 64],
        ..Surroundings::default()
    };
    let mut busy = UnitAi {
        current_action: ATTACK_ACTION,
        ..idle(2)
    };
    let mut side = StandSide {
        refusals: 0,
        tested: Vec::new(),
    };
    let mut fx = Vec::new();
    assert_eq!(
        busy.evasive_move(
            [12.0, 22.0, 1.5],
            (false, true),
            &at,
            &mut rng(),
            &mut side,
            &mut fx
        ),
        Ok(false)
    );
    assert_eq!((fx.len(), side.tested.len()), (0, 0));
    let mut u = idle(2);
    let mut side = StandSide {
        refusals: 1,
        tested: Vec::new(),
    };
    let mut r = rng();
    let mut expected = rng();
    let step = |r: &mut Well512, v: f32| {
        let up = r.rand15() & 1 != 0;
        let by = (r.rand15() & 3) as f32;
        if up { v + by } else { v - by }
    };
    let ex = step(&mut expected, 12.0);
    let ey = step(&mut expected, 22.0);
    assert_eq!(
        u.evasive_move(
            [12.0, 22.0, 1.5],
            (false, true),
            &at,
            &mut r,
            &mut side,
            &mut fx
        ),
        Ok(true)
    );
    assert_eq!(side.tested, vec![[12.0, 22.0], [ex, ey]]);
    assert_eq!(r.draws, 4);
    assert_eq!(
        fx,
        vec![Effect::Move(AiMove {
            point: [ex, ey, 1.5],
            range: 0.0,
            force: true,
            walk: false,
        })]
    );
    assert_eq!(
        (u.current_action, u.target_point),
        (MOVE_ACTION, [ex, ey, 1.5])
    );
    assert!(u.commanded);
    // a point past the far edge is held a tile in, below 0 at 0
    let mut u = idle(2);
    let mut fx = Vec::new();
    let mut side = StandSide {
        refusals: 0,
        tested: Vec::new(),
    };
    let _ = u.evasive_move(
        [70.0, -3.0, 0.0],
        (true, false),
        &at,
        &mut rng(),
        &mut side,
        &mut fx,
    );
    assert_eq!(first_move(&fx).map(|m| m.point), Some([63.0, 0.0, 0.0]));
}

/// Slot 0x148 (`FUN_140c4d0d0`): unforced, a unit AI with an action value takes no move off
/// a footprint; forced, or without one, the unit's slot 0x340 gets the move
/// ([`Effect::MoveAside`]) and the action value becomes 0x2bf, the old one the last.
#[test]
fn a_move_off_a_footprint_needs_force_or_no_action() {
    let m = |force| AiMove {
        point: [4.0, 5.0, 0.0],
        range: 0.0,
        force,
        walk: true,
    };
    let mut u = UnitAi {
        current_action: GATHER_ACTION,
        current_target: 40,
        ..idle(8)
    };
    let mut fx = Vec::new();
    assert!(!u.move_aside(m(false), &mut fx));
    assert_eq!((fx.len(), u.current_action), (0, GATHER_ACTION));
    assert!(u.move_aside(m(true), &mut fx));
    assert_eq!(fx, vec![Effect::MoveAside(m(true))]);
    assert_eq!(
        (u.last_action, u.current_action, u.current_target),
        (GATHER_ACTION, MOVE_ASIDE_ACTION, 40)
    );
    let mut idle_unit = idle(8);
    let mut fx = Vec::new();
    assert!(idle_unit.move_aside(m(false), &mut fx));
    assert_eq!(fx, vec![Effect::MoveAside(m(false))]);
}

/// Slot 0x130 (`FUN_140c55620`): unforced, a unit AI with an action value takes no move;
/// forced, the action value becomes the move's (the old one the last) and `FUN_140c64c10`
/// with −1 moves the target to the last and takes the point, leaving the target class.
#[test]
fn an_ai_move_needs_force_or_no_action_and_keeps_the_target_class() {
    let m = |force| AiMove {
        point: [4.0, 5.0, 0.5],
        range: 0.5,
        force,
        walk: true,
    };
    let mut u = UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 40,
        target_type: 9,
        last_target: 30,
        last_target_type: 4,
        ..idle(4)
    };
    let before = u.clone();
    assert_eq!(u.move_to(m(false)), None);
    assert_eq!(u, before);
    assert_eq!(u.move_to(m(true)), Some(m(true)));
    assert_eq!(
        (u.last_action, u.current_action),
        (ATTACK_ACTION, MOVE_ACTION)
    );
    assert_eq!(
        (
            u.last_target,
            u.last_target_type,
            u.current_target,
            u.target_type
        ),
        (40, 9, -1, 9)
    );
    assert_eq!(u.target_point, [4.0, 5.0, 0.5]);
}

/// `FUN_140c64b60`: leaving a patrol order (711, 733) redraws an adjusted idle timeout of
/// exactly 750 and caps the secondary timer at 750 by a signed compare (a timer read as
/// negative stays); with its third argument 0 the last order stays.
#[test]
fn the_order_setter_caps_by_a_signed_compare_and_may_keep_the_last_order() {
    let mut u = UnitAi {
        current_order: PATROL,
        last_order: 5,
        adjusted_idle_timeout: 3000,
        secondary_timer: 0x8000_0000,
        ..idle(13)
    };
    u.set_order_with(ATTACK_ORDER, false, &mut rng());
    assert_eq!((u.current_order, u.last_order), (ATTACK_ORDER, 5));
    assert_eq!(u.secondary_timer, 0x8000_0000);
    let mut u = UnitAi {
        current_order: ATTACK_MOVE,
        adjusted_idle_timeout: 750,
        idle_timeout: 3000,
        secondary_timer: 900,
        ..idle(13)
    };
    let mut r = rng();
    u.set_order(-1, &mut r);
    assert_eq!((u.current_order, u.last_order), (-1, ATTACK_MOVE));
    assert_eq!(u.secondary_timer, 750);
    assert_eq!(r.draws, 1);
}

/// The civilian's slot 73 (`FUN_140c21da0`): on the attack order without a target it stops
/// its action and ends the order (slot 0xb0 with 0: the state position takes its place);
/// with a target, or on another order, nothing.
#[test]
fn a_villager_on_the_attack_order_without_a_target_gives_it_up() {
    let at = Surroundings {
        position: [12.0, 13.0, 0.0],
        class: 4,
        ..Surroundings::default()
    };
    let mut u = UnitAi {
        current_order: ATTACK_ORDER,
        order_priority: 100,
        current_action: ATTACK_ACTION,
        ..idle(8)
    };
    let mut fx = Vec::new();
    u.civilian_misc(&at, &mut fx).unwrap();
    assert_eq!(fx, vec![Effect::Stop]);
    assert_eq!(
        (
            u.current_order,
            u.order_priority,
            u.last_order,
            u.current_action
        ),
        (-1, -1, ATTACK_ORDER, -1)
    );
    assert_eq!(u.state_position, [12.0, 13.0]);
    for (order, target) in [(ATTACK_ORDER, 40), (BUILD_ORDER, -1)] {
        let mut u = UnitAi {
            current_order: order,
            current_target: target,
            ..idle(8)
        };
        let mut fx = Vec::new();
        u.civilian_misc(&at, &mut fx).unwrap();
        assert_eq!((fx.len(), u.current_order), (0, order));
    }
}

/// The factory (`FUN_141215a90`): masters 1105, 1744 and 1746 by their id; then by class,
/// the classes without a unit AI, the civilian, the building (or the tower with a volley
/// missile, `+0x8d4`, and for class 52), the hunted animal, the soldier as the default and
/// the rest of the switch; class 10's type its master's combat level picks
/// (`FUN_140fde4f0`: the boar's for 1, else the lion's).
#[test]
fn the_factory_picks_the_unit_ai_type_by_master_and_class() {
    assert_eq!(factory_type(6, 1105, -1, 0), Some(0x12));
    assert_eq!(factory_type(6, 1744, -1, 0), Some(0x10));
    assert_eq!(factory_type(6, 1746, -1, 0), Some(0x10));
    let by_class = [
        (1, 0xe),
        (2, 0xa),
        (19, 0xa),
        (4, 8),
        (9, 2),
        (59, 2),
        (13, 0x10),
        (51, 0x10),
        (54, 0x10),
        (55, 0x10),
        (18, 9),
        (43, 9),
        (53, 9),
        (20, 0xb),
        (21, 0xc),
        (29, 0x13),
        (58, 0x13),
        (52, 0xf),
        (6, 0xd),
        (0, 0xd),
        (64, 0xd),
    ];
    for (class, ai_type) in by_class {
        assert_eq!(
            factory_type(class, 83, -1, 0),
            Some(ai_type),
            "class {class}"
        );
    }
    for class in [3, 27, 39, 49, 60] {
        assert_eq!(factory_type(class, 70, -1, 0), Some(17));
        assert_eq!(factory_type(class, 70, 5, 0), Some(0xf));
    }
    for class in [5, 7, 8, 11, 14, 15, 16, 30, 31, 32, 33, 34, 38, 42, 48, 63] {
        assert_eq!(factory_type(class, 83, -1, 0), None);
    }
    // class 10: the boar's for combat level 1, else the lion's
    assert_eq!(factory_type(10, 48, -1, 1), Some(BOAR_AI));
    assert_eq!(factory_type(10, 126, -1, 0), Some(5));
    assert_eq!(factory_type(10, 126, -1, 4), Some(5));
}

/// The building's cared classes (`FUN_140c11470`) are the soldier's in another order.
#[test]
fn the_building_cares_about_the_soldier_s_classes_in_its_own_order() {
    let mut building = BUILDING_CARED.to_vec();
    let mut soldier = SOLDIER_CARED.to_vec();
    assert_ne!(building, soldier);
    building.sort_unstable();
    soldier.sort_unstable();
    assert_eq!(building, soldier);
}

/// The hunted animal's damage notify (`FUN_140c26420`) for a human player's king (class 59):
/// the owner's alert (0x201, a human player's alert list, not held), the attacker joins the
/// set, the unit is under attack and runs from its attackers (slot 0x160; class 59 not
/// unconditionally, so only without an action value), by twice its speed away from them;
/// processing breaks off (3). A computer player's AI would hear the alert (not modelled).
#[test]
fn a_human_player_s_king_runs_from_its_attacker() {
    let mut u = UnitAi {
        hunted: None,
        ..idle(2)
    };
    let at = Surroundings {
        id: 500,
        position: [20.0, 30.0, 0.0],
        speed: 1.0,
        map: [64, 64],
        player: 1,
        class: CLASS_KING,
        state: 2,
        units: vec![UnitSeen {
            id: 77,
            class: 6,
            owner: 2,
            state: 2,
            target: Some(500),
            position: [22.0, 30.0, 0.0],
            ..UnitSeen::default()
        }],
        ..Surroundings::default()
    };
    let n = Notify::own(77, DAMAGED, [77, 40, 0]);
    let mut side = StandSide {
        refusals: 0,
        tested: Vec::new(),
    };
    let mut fx = Vec::new();
    let r = u.hunted_damaged(n, 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r, Ok(BREAK));
    assert_eq!(u.attackers, vec![77]);
    assert_eq!(
        fx,
        vec![Effect::Move(AiMove {
            point: [18.0, 30.0, 0.0],
            range: 0.0,
            force: true,
            walk: false,
        })]
    );
    let mut u = UnitAi {
        hunted: None,
        ..idle(2)
    };
    let r = u.hunted_damaged(n, 0, &at, true, &mut rng(), &mut side, &mut Vec::new());
    assert!(r.is_err());
}

/// `runAwayFromAttackers` (`FUN_140c5e900`) beyond the king's run: stand ground and no
/// attack stay (Gaia's run all the same), and so does a unit with an action value unless
/// unconditionally; an attacker the world does not hold is left out; attackers that cancel
/// out send it a random way (two draws of the AI stream); a defensive unit (stance 1) runs at
/// most 5 tiles from its state position. A computer player's siege unit, a computer's unit
/// attacked by class 22 and a computer's transport ship are not modelled.
#[test]
fn a_unit_runs_from_its_attackers_within_its_leash() {
    let seen = |id, class, x| UnitSeen {
        id,
        class,
        owner: 2,
        state: 2,
        target: Some(500),
        position: [x, 30.0, 0.0],
        ..UnitSeen::default()
    };
    let at = Surroundings {
        id: 500,
        position: [20.0, 30.0, 0.0],
        speed: 1.0,
        map: [64, 64],
        player: 1,
        class: 6,
        state: 2,
        units: vec![
            seen(77, 6, 22.0),
            seen(78, 6, 18.0),
            seen(79, CLASS_22, 22.0),
        ],
        ..Surroundings::default()
    };
    let run = |u: &mut UnitAi, at: &Surroundings, uncon, computer| {
        let mut side = StandSide {
            refusals: 0,
            tested: Vec::new(),
        };
        let mut fx = Vec::new();
        let got = u.run_away(uncon, at, computer, &mut rng(), &mut side, &mut fx);
        (got, fx)
    };
    let to = |point| {
        vec![Effect::Move(AiMove {
            point,
            range: 0.0,
            force: true,
            walk: false,
        })]
    };
    let runner = |attackers: Vec<i32>| UnitAi {
        attackers,
        ..idle(2)
    };
    for stance in [2, 3] {
        let mut u = UnitAi {
            stance,
            ..runner(vec![77])
        };
        assert_eq!(run(&mut u, &at, true, false), (Ok(false), vec![]));
        let gaia = Surroundings {
            player: 0,
            ..at.clone()
        };
        let mut u = UnitAi {
            stance,
            ..runner(vec![77])
        };
        assert_eq!(run(&mut u, &gaia, true, false).1, to([18.0, 30.0, 0.0]));
    }
    let mut u = UnitAi {
        current_action: 0x258,
        ..runner(vec![77])
    };
    assert_eq!(run(&mut u, &at, false, false), (Ok(false), vec![]));
    let mut u = runner(vec![77, 4242]);
    assert_eq!(run(&mut u, &at, true, false).1, to([18.0, 30.0, 0.0]));
    // 77 and 78 on either side: the way two draws give
    let mut e = rng();
    let mut part = || {
        let r = e.rand15() as i32 as f32;
        (r + r) / RUN_DRAW_SCALE - RUN_ONE
    };
    let (vx, vy) = (part(), part());
    let d = crate::math::fmath::sqrt(vy * vy + vx * vx);
    let away = [20.0 - (vx / d) * 2.0, 30.0 - (vy / d) * 2.0, 0.0];
    let mut u = runner(vec![77, 78]);
    assert_eq!(run(&mut u, &at, true, false).1, to(away));
    // a defensive unit with its state position 5 tiles east runs no farther than that
    let mut u = UnitAi {
        stance: 1,
        state_position: [25.0, 30.0],
        ..runner(vec![77])
    };
    assert_eq!(run(&mut u, &at, true, false).1, to([20.0, 30.0, 0.0]));
    assert!(run(&mut runner(vec![79]), &at, true, true).0.is_err());
    let ship = Surroundings {
        class: 20,
        ..at.clone()
    };
    assert!(run(&mut runner(vec![77]), &ship, true, true).0.is_err());
    assert!(run(&mut idle(SIEGE_AI), &at, true, true).0.is_err());
}

/// `FUN_140c64c10`: the target and its class become the last ones, the id and the point are
/// taken, and the class only when the world holds the object (else it stays).
#[test]
fn a_target_takes_its_class_only_from_an_object_the_world_holds() {
    let mut u = UnitAi {
        current_target: 40,
        target_type: 9,
        ..idle(13)
    };
    u.target_at(41, None, [1.0, 2.0, 3.0]);
    assert_eq!(
        (
            u.last_target,
            u.last_target_type,
            u.current_target,
            u.target_type
        ),
        (40, 9, 41, 9)
    );
    assert_eq!(u.target_point, [1.0, 2.0, 3.0]);
    u.target_at(42, Some(4), [5.0, 6.0, 0.0]);
    assert_eq!(
        (
            u.last_target,
            u.last_target_type,
            u.current_target,
            u.target_type
        ),
        (41, 9, 42, 4)
    );
}

/// Slot 0x38 (`FUN_140c556d0`) answers whether it took the notify: 0 outside state 2, for a
/// 699 from a unit that is not attacking it and for the invalidation of another action; 1
/// for a movement completion it ignores (not queued) and for one it queues.
#[test]
fn the_unit_ai_s_notify_answers_whether_it_took_it() {
    let mut u = UnitAi {
        current_action: GATHER_ACTION,
        attackers: vec![70],
        ..idle(8)
    };
    let n = |caller, kind, p0| Notify::own(caller, kind, [p0, 0, 0]);
    assert!(!u.accept(n(70, DAMAGED, 0), 0));
    assert!(!u.accept(n(71, NO_LONGER_ON_ME, 0), 2));
    assert!(!u.accept(n(70, ACTION_INVALIDATED, ATTACK_ACTION), 2));
    assert!(u.accept(n(70, ACTION_COMPLETED, MOVE_ACTION), 2));
    assert_eq!(u.notifies, Vec::new());
    assert!(u.accept(n(70, NO_LONGER_ON_ME, 0), 2));
    assert!(u.accept(n(70, ACTION_INVALIDATED, GATHER_ACTION), 2));
    assert_eq!(u.notifies.len(), 2);
}

/// `0x140aa2a70` in state 3: a unit without an AI asks for its path even in range and keeps
/// its attack move; one off the patrol orders ends in range and stops attack-moving.
#[test]
fn a_unit_off_patrol_asks_a_plain_path() {
    let ask = UnitAi::path_ask(None, [20.0, 20.0], [30.0, 20.0]);
    assert_eq!(
        ask,
        PathAsk {
            ends_in_range: false,
            ..PathAsk::default()
        }
    );
    let mut u = idle(13);
    let ask = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [30.0, 20.0]);
    assert_eq!(
        ask,
        PathAsk {
            attack_move: Some(false),
            ..PathAsk::default()
        }
    );
    // a saved patrol off the patrol orders stays, and so does the attack move
    u.patrol_path = Some(PatrolPath::default());
    let ask = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [30.0, 20.0]);
    assert_eq!((ask.attack_move, u.patrol_path.is_some()), (None, true));
}

/// `0x140aa2a70` in state 3 on a patrol order: the path loops; without a saved patrol it goes
/// back and forth from 3 tiles (`SETBE`: a NaN distance too); order 733 attack-moves.
#[test]
fn a_patrol_order_s_path_loops_and_turns_back_from_three_tiles() {
    let mut u = UnitAi {
        current_order: PATROL,
        ..idle(13)
    };
    let ask = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [22.9, 20.0]);
    assert_eq!(
        ask,
        PathAsk {
            ends_in_range: false,
            patrol: true,
            back_and_forth: false,
            resume: -1,
            attack_move: Some(false),
        }
    );
    let far = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [20.0, 23.0]);
    assert!(far.back_and_forth);
    let nan = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [f32::NAN, 23.0]);
    assert!(nan.back_and_forth);
    u.current_order = ATTACK_MOVE;
    let attack = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [22.9, 20.0]);
    assert_eq!((attack.patrol, attack.attack_move), (true, Some(true)));
}

/// `0x140aa2a70` in state 3 on a patrol order with a saved patrol: its waypoint and its back
/// and forth (`getPatrolLinear`, flags bit 1) are the path's, and it goes
/// (`zeroPatrolInformation`), which then stops the attack move.
#[test]
fn a_saved_patrol_gives_its_waypoint_to_the_path_and_goes() {
    let mut u = UnitAi {
        current_order: PATROL,
        patrol_path: Some(PatrolPath {
            flags: 3,
            ..PatrolPath::default()
        }),
        patrol_waypoint: 2,
        ..idle(13)
    };
    let ask = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [20.5, 20.0]);
    assert_eq!(
        (ask.resume, ask.back_and_forth, ask.attack_move),
        (2, true, Some(false))
    );
    assert_eq!((u.patrol_path, u.patrol_waypoint), (None, 0));
}

/// A human player's soldier walking its patrol (711 with the move action, its `MoveTo` the
/// top action) as the full update finds it, its secondary timer `secondary`.
fn patroller(secondary: u32) -> (UnitAi, Surroundings) {
    let u = UnitAi {
        current_order: PATROL,
        current_action: MOVE_ACTION,
        idle_timeout: 3000,
        adjusted_idle_timeout: 750,
        secondary_timer: secondary,
        ..idle(SOLDIER_AI)
    };
    let at = Surroundings {
        position: [20.0, 20.0, 0.0],
        player: 1,
        kind: 70,
        state: 2,
        size: [0.2, 0.2],
        los: 4.0,
        weapon_range: 1.0,
        busy: true,
        ..Surroundings::default()
    };
    (u, at)
}

/// The idle part's patrol branch (`0x140c67f0b`) while the secondary timer runs: nothing
/// but the timer's count down; the busy check lets the walking patrol through.
#[test]
fn a_patrol_waits_for_its_secondary_timer() {
    let (mut u, at) = patroller(300);
    let mut side = SoldierSide {
        found: 77,
        attacks: Vec::new(),
        patrol: None,
        refuses: false,
    };
    let fx = u.update(28, 0, false, &at, &mut rng(), &mut side).unwrap();
    assert_eq!(fx, vec![]);
    assert_eq!((u.secondary_timer, u.current_order), (272, PATROL));
    assert_eq!(side.attacks, Vec::<(i32, bool)>::new());
}

/// The patrol branch with its secondary timer run out: it restarts at 750 (the adjusted
/// idle timeout of 750 redrawn), processIdle's search finds 77: the patrol is saved (the
/// unit's slot 0x468), the attack command given, and the order goes without its history
/// (`FUN_140c64b60` with -1 and 0).
#[test]
fn a_patrol_saves_itself_and_attacks_what_it_finds() {
    let (mut u, at) = patroller(0);
    u.last_order = 9;
    let saved = PatrolPath {
        flags: 3,
        ..PatrolPath::default()
    };
    let mut side = SoldierSide {
        found: 77,
        attacks: Vec::new(),
        patrol: Some((saved.clone(), 2)),
        refuses: false,
    };
    let mut r = rng();
    let _ = u.update(28, 0, false, &at, &mut r, &mut side).unwrap();
    assert_eq!(side.attacks, vec![(77, true)]);
    assert_eq!((u.current_order, u.last_order), (-1, 9));
    assert_eq!((u.patrol_path, u.patrol_waypoint), (Some(saved), 2));
    assert_eq!(u.secondary_timer, 750);
    assert_ne!(u.adjusted_idle_timeout, 750);
    assert!(0 < r.draws);
}

/// The patrol branch passes over processIdle while the top action's target is a building
/// (slot 400); a unit target it does not.
#[test]
fn a_patrol_attacking_a_building_keeps_at_it() {
    let (mut u, mut at) = patroller(0);
    at.top_target = Some(TopTarget {
        id: 50,
        kind: 80,
        position: [30.0, 30.0, 0.0],
        radius: [1.0, 1.0],
    });
    let mut side = SoldierSide {
        found: 77,
        attacks: Vec::new(),
        patrol: None,
        refuses: false,
    };
    let _ = u.update(28, 0, false, &at, &mut rng(), &mut side).unwrap();
    assert_eq!(side.attacks, Vec::<(i32, bool)>::new());
    assert_eq!(u.secondary_timer, 750);
}

/// The soldier's idle check (`FUN_140c20db0`) takes its top action's target within its
/// range (slot 400, slot 0x1e8: the footprints' squared gap between the least range and the
/// weapon range squared) without a search; out of range it searches.
#[test]
fn a_soldier_s_idle_check_attacks_its_top_action_s_target_in_range() {
    let (mut u, mut at) = patroller(0);
    u.current_order = -1;
    let near = TopTarget {
        id: 50,
        kind: 70,
        position: [21.0, 20.0, 0.0],
        radius: [0.2, 0.2],
    };
    at.top_target = Some(near);
    let mut side = SoldierSide {
        found: 77,
        attacks: Vec::new(),
        patrol: None,
        refuses: false,
    };
    assert_eq!(u.top_target_in_range(&at, &[], &mut side), Ok(Some(50)));
    at.min_range = 1.0;
    assert_eq!(u.top_target_in_range(&at, &[], &mut side), Ok(None));
    at.min_range = 0.0;
    let mut fx = Vec::new();
    assert_eq!(
        u.soldier_idle(&at, false, &mut rng(), &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(side.attacks, vec![(50, true)]);
    // too far: the search's 77
    let (mut u, mut at) = patroller(0);
    u.current_order = -1;
    at.top_target = Some(TopTarget {
        position: [25.0, 20.0, 0.0],
        ..near
    });
    side.attacks.clear();
    assert_eq!(
        u.soldier_idle(&at, false, &mut rng(), &mut side, &mut Vec::new()),
        Ok(true)
    );
    assert_eq!(side.attacks, vec![(77, true)]);
}

/// The soldier's idle check's other ways (`FUN_140c20db0`): master 706 checks nothing; with
/// retargeting off a current target the world holds is attacked without a search, the
/// order kept; a failed attack on a patrol drops the saved patrol and stops on the order (an
/// attack on its own current target with an action takes nothing); a computer player's
/// check and a healer's ally search are not modelled.
#[test]
fn a_soldier_s_idle_check_holds_its_target_with_retargeting_off() {
    let side = |found| SoldierSide {
        found,
        attacks: Vec::new(),
        patrol: None,
        refuses: false,
    };
    let idle = |u: &mut UnitAi, at: &Surroundings, side: &mut SoldierSide, computer| {
        u.soldier_idle(at, computer, &mut rng(), side, &mut Vec::new())
    };
    let (mut u, mut at) = patroller(0);
    at.master = SOLDIER_IDLE_MASTER;
    let mut s = side(77);
    assert_eq!(idle(&mut u, &at, &mut s, false), Ok(false));
    assert_eq!(s.attacks, []);
    let (mut u, mut at) = patroller(0);
    (u.current_order, u.retarget_disabled, u.current_target) = (-1, 1, 60);
    at.target = Some(Seen::default());
    let mut s = side(77);
    assert_eq!(idle(&mut u, &at, &mut s, false), Ok(true));
    assert_eq!((s.attacks, u.current_order), (vec![(60, true)], -1));
    let (mut u, at) = patroller(0);
    assert!(idle(&mut u, &at, &mut side(77), true).is_err());
    let (mut u, at) = patroller(0);
    let mut s = SoldierSide {
        refuses: true,
        patrol: Some((PatrolPath::default(), 3)),
        ..side(77)
    };
    assert_eq!(idle(&mut u, &at, &mut s, false), Ok(false));
    assert_eq!((&u.patrol_path, u.patrol_waypoint), (&None, 0));
    let (mut u, mut at) = patroller(0);
    u.current_order = -1;
    at.task_0x69 = true;
    assert!(idle(&mut u, &at, &mut side(-1), false).is_err());
}

/// A stop earlier in the update (a failed Enter's notify, slot 0xb0) emptied the action list
/// at once: the soldier's idle check finds no top action's target, in range as it was at
/// the update's start, and searches (426750657's scout inside its transport, update 467);
/// an effect that leaves the actions does not change it.
#[test]
fn a_soldier_stopped_in_the_update_finds_no_top_action_s_target() {
    let (mut u, mut at) = patroller(0);
    u.current_order = -1;
    at.top_target = Some(TopTarget {
        id: 50,
        kind: 70,
        position: [21.0, 20.0, 0.0],
        radius: [0.2, 0.2],
    });
    let mut side = SoldierSide {
        found: 77,
        attacks: Vec::new(),
        patrol: None,
        refuses: false,
    };
    let mut read = |fx: &[Effect]| u.top_target_in_range(&at, fx, &mut side);
    assert_eq!(read(&[]), Ok(Some(50)));
    assert_eq!(read(&[Effect::CapHumanOrder(5000)]), Ok(Some(50)));
    assert_eq!(read(&[Effect::Stop]), Ok(None));
    assert_eq!(read(&[Effect::Stop, Effect::CapHumanOrder(5000)]), Ok(None));
}

/// The soldier's idle check with nothing to attack and a saved patrol, in no group:
/// `restorePatrolInformation` gives the unit its walk back onto the patrol, the order 711
/// (733 for a unit that attack-moves) and a secondary timer of 750; processIdle produced.
#[test]
fn a_soldier_with_nothing_to_attack_goes_back_to_its_patrol() {
    let (mut u, mut at) = patroller(0);
    u.current_order = -1;
    u.current_action = -1;
    u.patrol_path = Some(PatrolPath::default());
    at.busy = false;
    let mut side = SoldierSide {
        found: -1,
        attacks: Vec::new(),
        patrol: None,
        refuses: false,
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.soldier_idle(&at, false, &mut rng(), &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(
        fx,
        vec![
            Effect::CapHumanOrder(HUMAN_ORDER_CAP),
            Effect::RestorePatrol
        ]
    );
    assert_eq!((u.current_order, u.secondary_timer), (PATROL, 750));
    u.current_order = -1;
    at.attack_moving = true;
    assert_eq!(
        u.soldier_idle(&at, false, &mut rng(), &mut side, &mut Vec::new()),
        Ok(true)
    );
    assert_eq!(u.current_order, ATTACK_MOVE);
}

/// The order check of a unit no longer busy: the attack move (733) ends, its timers reset
/// (an adjusted idle timeout of 750 redrawn, the secondary timer capped at 750); the patrol
/// order (711) stays.
#[test]
fn an_attack_move_not_busy_ends_and_a_patrol_stays() {
    let at = Surroundings::default();
    let mut u = UnitAi {
        current_order: ATTACK_MOVE,
        idle_timeout: 3000,
        adjusted_idle_timeout: 750,
        secondary_timer: 900,
        ..idle(SOLDIER_AI)
    };
    u.check_order(&at, &mut rng());
    assert_eq!((u.current_order, u.secondary_timer), (-1, 750));
    assert_ne!(u.adjusted_idle_timeout, 750);
    let mut u = UnitAi {
        current_order: PATROL,
        ..idle(SOLDIER_AI)
    };
    u.check_order(&at, &mut rng());
    assert_eq!(u.current_order, PATROL);
}

/// Slot 0xb0 (`FUN_140c64ed0`) on a patrol order: the order ends even when the stop keeps
/// orders, its secondary timer capped at 750; with an adjusted idle timeout of exactly 750
/// the redraw (the AI stream's) is not modelled here.
#[test]
fn a_stop_ends_a_patrol_order_with_its_secondary_timer_capped() {
    let mut u = UnitAi {
        current_order: PATROL,
        current_action: MOVE_ACTION,
        adjusted_idle_timeout: 2600,
        secondary_timer: 900,
        ..idle(SOLDIER_AI)
    };
    let mut fx = Vec::new();
    u.stop_with(true, Some(([3.0, 4.0], 6)), &mut fx).unwrap();
    assert_eq!(
        (u.current_order, u.last_order, u.secondary_timer),
        (-1, PATROL, 750)
    );
    assert_eq!((fx, u.state_position), (vec![Effect::Stop], [3.0, 4.0]));
    let mut u = UnitAi {
        current_order: ATTACK_MOVE,
        adjusted_idle_timeout: 750,
        ..idle(SOLDIER_AI)
    };
    assert!(u.stop_with(true, None, &mut Vec::new()).is_err());
}

/// The order's tail (`LAB_140c5d611`) and the Build's end (`FUN_140c64b60` with −1): leaving
/// a patrol order caps the secondary timer; staying on it does not.
#[test]
fn leaving_a_patrol_order_caps_its_secondary_timer() {
    let order = |order| QueuedOrder {
        order,
        priority: 100,
        ..QueuedOrder::default()
    };
    let patrolling = || UnitAi {
        current_order: PATROL,
        adjusted_idle_timeout: 2600,
        secondary_timer: 900,
        ..idle(SOLDIER_AI)
    };
    let tail =
        |u: &mut UnitAi, o: QueuedOrder| u.order_tail(o.order, o.priority, 6, false, &mut rng());
    let mut u = patrolling();
    assert_eq!(tail(&mut u, order(MOVE_ORDER)), 1);
    assert_eq!((u.current_order, u.secondary_timer), (MOVE_ORDER, 750));
    let mut u = patrolling();
    assert_eq!(tail(&mut u, order(PATROL)), 1);
    assert_eq!(u.secondary_timer, 900);
    let mut u = patrolling();
    u.end_order_and_action().unwrap();
    assert_eq!(
        (u.current_order, u.last_order, u.secondary_timer),
        (-1, PATROL, 750)
    );
}

/// A unit AI with an order on a target ready in its queue, its idle timer, look-around timer
/// and retarget timer all past their timeouts.
fn ordered(ai_type: u32) -> UnitAi {
    UnitAi {
        orders: vec![QueuedOrder {
            order: ORDER_ON_TARGET,
            priority: 100,
            target: 77,
            ready: true,
            ..QueuedOrder::default()
        }],
        idle_timer: 5000,
        look_around_timer: 2000,
        last_retarget_time: 0,
        retarget_timer: 900,
        ..idle(ai_type)
    }
}

/// `0x140c66b10` runs the order on a target inside the update (slot 0x230's case 0x2db), so
/// the update stops there: the order and what the rest of the update reads of its start
/// ([`AfterOrder`]: its `busy` and group) go to the caller; slot 73, the idle part and the
/// look-around check have not run (no draw, the timers as they grew).
#[test]
fn a_full_update_stops_at_its_order_on_a_target() {
    let mut u = ordered(SOLDIER_AI);
    let order = u.orders[0];
    let mut r = rng();
    // the world holds the order's target
    let at = Surroundings {
        units: vec![UnitSeen {
            id: 77,
            ..UnitSeen::default()
        }],
        ..Surroundings::default()
    };
    let fx = u
        .update(100, 10_000, false, &at, &mut r, &mut no_search)
        .unwrap();
    assert_eq!(
        fx,
        vec![
            Effect::OrderOnTarget(order),
            Effect::AfterOrder(AfterOrder {
                busy: false,
                group: None,
            }),
        ]
    );
    assert_eq!(r.draws, 0);
    assert_eq!(
        (
            u.idle_timer,
            u.look_around_timer,
            u.retarget_timer,
            u.orders.len()
        ),
        (5100, 2100, 900, 0)
    );
}

/// The update's rest after the order (`0x140c66b10` from slot 73 on): the retarget check
/// reads the order's attack (its timer redrawn, then a human unit in an attack's work state
/// stops there); the idle part sees the action, so an action of a player above Gaia keeps the
/// idle timer at 0; the look-around check redraws its timeout.
#[test]
fn the_rest_after_an_order_that_gave_an_action_reads_it() {
    let mut u = ordered(SOLDIER_AI);
    u.orders.clear();
    u.current_order = ATTACK_ORDER;
    u.current_action = ATTACK_ACTION;
    u.current_target = 77;
    let at = Surroundings {
        player: 1,
        target: Some(Seen {
            owner: 2,
            class: 6,
            ..Seen::default()
        }),
        action_state: Some(6),
        ..Surroundings::default()
    };
    let mut r = rng();
    let fx = u
        .after_order(
            AfterOrder::default(),
            10_000,
            false,
            &at,
            &mut r,
            &mut no_search,
        )
        .unwrap();
    let mut expected = rng();
    let (retarget, look) = (expected.rand15(), expected.rand15());
    assert_eq!(fx, Vec::<Effect>::new());
    assert_eq!(
        (
            u.last_retarget_time,
            u.retarget_timer,
            u.idle_timer,
            u.look_around_timer,
            u.look_around_timeout,
            r.draws
        ),
        (10_000, 750 + retarget % 500, 0, 0, 4500 + look % 1000, 2)
    );
}

/// An order that gave the AI no action (no task on its target): the idle timer past its
/// timeout restarts and slot 72 runs with 1 (the villager's: its cap of the last human order
/// time). With the update's `busy` and a target the order gave, it does not; the idle part's
/// end then keeps the timer of a player above Gaia at 0.
#[test]
fn the_rest_after_an_order_that_gave_no_action_runs_the_idle_check() {
    let mut u = ordered(CIVILIAN);
    u.orders.clear();
    let at = Surroundings {
        player: 1,
        ..Surroundings::default()
    };
    let mut r = rng();
    let fx = u
        .after_order(
            AfterOrder::default(),
            10_000,
            false,
            &at,
            &mut r,
            &mut no_search,
        )
        .unwrap();
    assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
    assert_eq!((u.idle_timer, r.draws), (0, 1));
    let mut u = ordered(CIVILIAN);
    u.orders.clear();
    u.current_target = 77;
    let busy = AfterOrder {
        busy: true,
        group: None,
    };
    let fx = u
        .after_order(busy, 10_000, false, &at, &mut rng(), &mut no_search)
        .unwrap();
    assert_eq!((fx, u.idle_timer), (Vec::<Effect>::new(), 0));
    // Gaia's unit keeps its timer
    let mut u = ordered(CIVILIAN);
    u.orders.clear();
    u.current_target = 77;
    let gaia = Surroundings::default();
    let _ = u
        .after_order(busy, 10_000, false, &gaia, &mut rng(), &mut no_search)
        .unwrap();
    assert_eq!(u.idle_timer, 5000);
}

/// A human player's transport ship at (20, 30) attacked by unit 77 two tiles east of it,
/// already among its attackers.
fn attacked_transport() -> (UnitAi, Surroundings) {
    let u = UnitAi {
        attackers: vec![77],
        ..idle(11)
    };
    let at = Surroundings {
        id: 500,
        position: [20.0, 30.0, 0.0],
        speed: 1.0,
        map: [64, 64],
        player: 1,
        class: 20,
        state: 2,
        units: vec![UnitSeen {
            id: 77,
            class: 22,
            owner: 2,
            state: 2,
            target: Some(500),
            position: [22.0, 30.0, 0.0],
            ..UnitSeen::default()
        }],
        ..Surroundings::default()
    };
    (u, at)
}

/// The transport ship's handler (`FUN_140c2a580`): a notify other than damage and a
/// missile's warning is the base handler's, as it answers it.
#[test]
fn the_transport_ship_passes_other_notifies_to_the_base_handler() {
    let (mut u, at) = attacked_transport();
    u.current_action = MOVE_ACTION;
    u.current_order = MOVE_ORDER;
    let mut base = u.clone();
    let n = Notify::own(500, ACTION_COMPLETED, [MOVE_ACTION, 0, 0]);
    let (mut fx, mut base_fx) = (Vec::new(), Vec::new());
    let r = u.handle(
        n,
        0,
        &at,
        false,
        &mut rng(),
        &mut Recorder::default(),
        &mut fx,
    );
    let b = base.base_notify(
        n,
        0,
        &at,
        false,
        &mut rng(),
        &mut Recorder::default(),
        &mut base_fx,
    );
    assert_eq!((r, fx), (b, base_fx));
    assert_eq!(u, base);
}

/// Its damage notify: the attacker it knows joins no one new (no call for help), the unit is
/// under attack; without an order or an action it runs from its attackers by twice its speed
/// (slot 0x160 with 1) and breaks off (3). With an order it ignores the damage (2). At no hit
/// points left it gives up (4). A computer player's AI would hear the alert (not modelled).
#[test]
fn a_transport_ship_runs_from_its_attackers_without_an_order() {
    let (mut u, at) = attacked_transport();
    let n = Notify::own(77, DAMAGED, [77, 40, 0]);
    let mut side = StandSide {
        refusals: 0,
        tested: Vec::new(),
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(u.attackers, vec![77]);
    assert_eq!(
        fx,
        vec![Effect::Move(AiMove {
            point: [18.0, 30.0, 0.0],
            range: 0.0,
            force: true,
            walk: false,
        })]
    );
    let (mut u, at) = attacked_transport();
    u.current_order = MOVE_ORDER;
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!(fx, Vec::<Effect>::new());
    // busy with its top action in state 2 it runs too; in another state it stays
    let (mut u, mut at) = attacked_transport();
    at.busy = true;
    at.action_state = Some(4);
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(fx, Vec::<Effect>::new());
    let (mut u, at) = attacked_transport();
    let dead = Notify::own(77, DAMAGED, [77, 0, 0]);
    assert_eq!(
        u.handle(dead, 0, &at, false, &mut rng(), &mut side, &mut Vec::new()),
        Ok(DEAD)
    );
    let (mut u, at) = attacked_transport();
    assert!(
        u.handle(n, 0, &at, true, &mut rng(), &mut side, &mut Vec::new())
            .is_err()
    );
    // a new attacker that is not its target: the call for help (AskForHelp) looks the
    // attacker up in the world first, which this side does not hold
    let (mut u, at) = attacked_transport();
    u.attackers.clear();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut Vec::new()),
        Err(GenieError::unsupported(
            "an object's look without the world"
        ))
    );
}

/// A missile's warning (`0x20f`): one draw, `r * 100 / 0x7fff` below the owner's
/// `chance_to_dodge_missiles` dodges it (2); else it is damage.
#[test]
fn a_transport_ship_dodges_a_missile_s_warning_by_its_owner_s_chance() {
    let r = rng().rand15() as i32 * 100 / 0x7fff;
    let warning = Notify::own(77, MISSILE_WARNING, [77, 40, 0]);
    let mut side = StandSide {
        refusals: 0,
        tested: Vec::new(),
    };
    let (mut u, mut at) = attacked_transport();
    at.dodge_chance = u8::try_from(r + 1).unwrap();
    let mut ai = rng();
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(warning, 0, &at, false, &mut ai, &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!((ai.draws, fx.len()), (1, 0));
    let (mut u, mut at) = attacked_transport();
    at.dodge_chance = u8::try_from(r).unwrap();
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(warning, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(fx.len(), 1);
}

/// The siege tower (0x12): its idle check (`FUN_140c20690`) only caps the last human order
/// time; its damage notify (`FUN_140c28400`) is the transport ship's, but a human player's
/// tower in a group that is not idle ignores the damage (2), and one in an idle group runs.
#[test]
fn a_siege_tower_in_a_busy_group_ignores_its_damage() {
    let mut u = idle(SIEGE_TOWER_AI);
    let mut fx = Vec::new();
    let mut r = rng();
    assert_eq!(
        u.process_idle(
            &Surroundings::default(),
            false,
            &mut r,
            &mut Recorder::default(),
            &mut fx
        ),
        Ok(false)
    );
    assert_eq!(
        (fx, r.draws),
        (vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)], 0)
    );
    let n = Notify::own(77, DAMAGED, [77, 40, 0]);
    let mut side = StandSide {
        refusals: 0,
        tested: Vec::new(),
    };
    let tower = |idle_group: bool| {
        let (mut u, mut at) = attacked_transport();
        u.ai_type = SIEGE_TOWER_AI;
        at.group = Some(GroupState {
            idle: idle_group,
            ..GroupState::default()
        });
        (u, at)
    };
    let (mut u, at) = tower(false);
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!((fx.len(), u.attackers.clone()), (0, vec![77]));
    let (mut u, at) = tower(true);
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(fx.len(), 1);
}

/// `AttackObject` (slot 0xb8) of the types whose slot 0xa0 is the base's (`FUN_140c48400`,
/// anything): the ranged units', the ships', the artifact's and the siege tower's attack a
/// unit the world holds; the AI takes the attack and the target.
#[test]
fn the_base_attack_check_lets_every_type_without_its_own_attack() {
    for ai_type in [6, 7, 10, 11, 12, ARTIFACT_AI, SIEGE_TOWER_AI] {
        let mut u = idle(ai_type);
        let at = Surroundings {
            units: vec![UnitSeen {
                id: 77,
                class: 6,
                position: [3.0, 4.0, 0.0],
                ..UnitSeen::default()
            }],
            ..Surroundings::default()
        };
        let mut side = Recorder::default();
        let mut fx = Vec::new();
        assert_eq!(
            u.attack_object(77, true, false, &at, &mut side, &mut fx),
            Ok(true),
            "type {ai_type}"
        );
        assert_eq!(
            (side.attacks, u.current_action, u.current_target),
            (vec![(77, true)], ATTACK_ACTION, 77)
        );
    }
}

/// A ship's world: its searches answer from `found` (one answer per call, in order) and are
/// recorded; the objects it names stand at their places; every point can be stood on and
/// every target reached within `reach`.
#[derive(Default)]
struct ShipSide {
    found: Vec<Found>,
    searched: Vec<(f32, Vec<i32>, Pick)>,
    units: Vec<UnitSeen>,
    reach: bool,
}

impl UnitSide for ShipSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError> {
        self.searched
            .push((search.radius, search.classes.to_vec(), search.pick));
        Ok(if self.found.is_empty() {
            Found::NONE
        } else {
            self.found.remove(0)
        })
    }
    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Err(GenieError::unsupported("no attack here"))
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn can_stand(&mut self, _: [f32; 2]) -> Result<bool, GenieError> {
        Ok(true)
    }
    fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
        Ok(self.units.iter().find(|u| u.id == id).copied())
    }
    fn reaches(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
        Ok(self.reach)
    }
}

/// The ships' idle check (`FUN_140c21930`, `FUN_140c1eab0`): a human player's only caps the
/// last human order time and searches nothing (5); a computer player's fishing ship looks for
/// the closest object of its classes, its radius grown by 7, and moves away from it by seven
/// times its speed (6); a trade cart searches its own classes; finding nothing, 5.
#[test]
fn a_computer_player_s_ship_evades_the_closest_object_it_sees() {
    let at = Surroundings {
        id: 500,
        position: [20.0, 30.0, 0.0],
        speed: 1.0,
        map: [64, 64],
        los: 4.0,
        player: 1,
        class: 21,
        ..Surroundings::default()
    };
    let mut u = idle(FISHING_AI);
    let mut fx = Vec::new();
    let mut side = ShipSide::default();
    assert_eq!(
        u.process_idle(&at, false, &mut rng(), &mut side, &mut fx),
        Ok(false)
    );
    assert_eq!(
        (fx, side.searched.len()),
        (vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)], 0)
    );
    let mut side = ShipSide {
        found: vec![Found {
            id: 77,
            ..Found::NONE
        }],
        units: vec![UnitSeen {
            id: 77,
            position: [22.0, 30.0, 0.0],
            ..UnitSeen::default()
        }],
        ..ShipSide::default()
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.process_idle(&at, true, &mut rng(), &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(
        fx,
        vec![
            Effect::CapHumanOrder(HUMAN_ORDER_CAP),
            Effect::Move(AiMove {
                point: [13.0, 30.0, 0.0],
                range: 0.0,
                force: true,
                walk: false,
            }),
        ]
    );
    let (radius, classes, pick) = side.searched[0].clone();
    assert_eq!((radius, classes), (12.0, FISHING_CARED.to_vec()));
    assert!(matches!(
        pick,
        Pick::Closest {
            group: -1,
            relation: -1,
            reach: true,
            ..
        }
    ));
    // a trade cart (class 19) finding nothing
    let mut cart = idle(TRADE_AI);
    let mut side = ShipSide::default();
    let at = Surroundings { class: 19, ..at };
    assert_eq!(
        cart.process_idle(&at, true, &mut rng(), &mut side, &mut Vec::new()),
        Ok(false)
    );
    assert_eq!(side.searched[0].0, 5.0);
    assert_eq!(side.searched[0].1, TRADE_CART_CARED.to_vec());
}

/// The trade ship's case 0x202 (`FUN_140c29bf0`) off its trade and explore actions: the
/// target goes, the action stops, and the processing goes on (2).
#[test]
fn a_trade_ship_whose_action_needs_a_search_stops() {
    let (mut u, at) = attacked_transport();
    u.ai_type = TRADE_AI;
    u.current_action = MOVE_ACTION;
    u.current_target = 12;
    let n = Notify::own(500, REQUIRES_SEARCH, [MOVE_ACTION, 0, 0]);
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(
            n,
            0,
            &at,
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut fx
        ),
        Ok(GO_ON)
    );
    assert_eq!((u.current_action, u.current_target), (-1, -1));
    assert_eq!(fx, vec![Effect::Stop]);
    for action in [TRADE_ACTION, EXPLORE_ACTION] {
        let (mut u, at) = attacked_transport();
        u.ai_type = TRADE_AI;
        u.current_action = action;
        assert!(
            u.handle(
                n,
                0,
                &at,
                false,
                &mut rng(),
                &mut Recorder::default(),
                &mut fx
            )
            .is_err()
        );
    }
}

/// The trade ship's damage notify (`FUN_140c29bf0`): an attacker it knows raises no alert
/// and no call for help; without an order, off the move action, the action stops and it
/// runs from its attackers (3); on the move action it runs on (no stop).
#[test]
fn a_trade_ship_stops_and_runs_from_its_attackers() {
    let n = Notify::own(77, DAMAGED, [77, 40, 0]);
    let (mut u, at) = attacked_transport();
    u.ai_type = TRADE_AI;
    u.current_action = 0x261;
    let mut fx = Vec::new();
    let mut side = ShipSide::default();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(fx.first(), Some(&Effect::Stop));
    assert_eq!(fx.len(), 2);
    assert_eq!(u.orders, Vec::new());
    let (mut u, at) = attacked_transport();
    u.ai_type = TRADE_AI;
    u.current_action = MOVE_ACTION;
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(u.current_action, MOVE_ACTION);
    assert!(!fx.contains(&Effect::Stop));
}

/// The fishing ship's damage notify (`FUN_140c25ca0`): on the move action it neither stops
/// nor runs (3, nothing); off it, it stops and runs.
#[test]
fn a_fishing_ship_on_the_move_takes_its_damage_without_running() {
    let n = Notify::own(77, DAMAGED, [77, 40, 0]);
    let (mut u, at) = attacked_transport();
    u.ai_type = FISHING_AI;
    u.current_action = MOVE_ACTION;
    let mut fx = Vec::new();
    let mut side = ShipSide::default();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(fx, Vec::new());
    let (mut u, at) = attacked_transport();
    u.ai_type = FISHING_AI;
    u.current_action = GATHER_ACTION;
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(fx.first(), Some(&Effect::Stop));
    assert_eq!(fx.len(), 2);
}

/// The fishing ship's builder search (`FUN_140c25ca0`, case 0x202 on its build): its own
/// foundations (relation 3, state 0) of the classes 3, 27, 39, 52 and 49, in that order;
/// the nearest by `ClosestObject`'s distance (a tie keeps the first) that it reaches it
/// builds (3). Reaching none, a human player's ship goes on (2); a computer player's posts
/// itself its build's completion.
#[test]
fn a_fishing_ship_builds_the_nearest_foundation_it_reaches() {
    let at = Surroundings {
        id: 500,
        position: [20.0, 30.0, 0.0],
        los: 4.0,
        player: 1,
        state: 2,
        ..Surroundings::default()
    };
    let found = |id, distance| Found {
        id,
        distance,
        ..Found::NONE
    };
    let mut u = UnitAi {
        current_action: BUILD_ACTION,
        ..idle(FISHING_AI)
    };
    let mut side = ShipSide {
        found: vec![
            found(-1, -1),
            found(41, 6),
            found(42, 3),
            found(43, 3),
            Found::NONE,
        ],
        reach: true,
        ..ShipSide::default()
    };
    let n = Notify::own(500, REQUIRES_SEARCH, [BUILD_ACTION, 0, 0]);
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(BREAK)
    );
    assert_eq!(fx, vec![Effect::BuildObject(42)]);
    let groups: Vec<i32> = side
        .searched
        .iter()
        .map(|(_, _, p)| match p {
            Pick::Closest {
                group,
                relation: 3,
                state: 0,
                ..
            } => *group,
            _ => -9,
        })
        .collect();
    assert_eq!(groups, FISHING_BUILDS.to_vec());
    // out of reach: the human's goes on, the computer's posts its completion
    let mut side = ShipSide {
        found: vec![found(41, 6)],
        ..ShipSide::default()
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
        Ok(GO_ON)
    );
    assert_eq!((fx.len(), u.buffered.len(), u.notifies.len()), (0, 0, 0));
    let mut side = ShipSide::default();
    assert_eq!(
        u.handle(n, 0, &at, true, &mut rng(), &mut side, &mut Vec::new()),
        Ok(GO_ON)
    );
    assert_eq!(
        u.notifies,
        vec![Notify::own(500, ACTION_COMPLETED, [BUILD_ACTION, 0, 0])]
    );
}

/// A siege unit of player 1 (a mangonel: class 13, master 280, range 7, radii 0.6) at its
/// state position.
const MANGONEL: Attacker = Attacker {
    ai_type: SIEGE_AI,
    player: 1,
    kind: 70,
    class: 13,
    master: 280,
    missile: 656,
    range: 7.0,
    los: 9.0,
    position: [20.0, 20.0, 0.0],
    radius: [0.6, 0.6],
    speed: 0.6,
    ..TC
};

/// `FUN_14120e4e0` for a type-70 attacker: priority 3 spares every class but its six; a
/// ranged attacker with a blast whose level has neither bit 2 nor bit 3 would scan the blast
/// for friends (not modelled); a melee attacker, one without a blast, or a blast level with
/// either bit spares nobody.
#[test]
fn an_attacker_spares_by_its_priority_and_its_blast() {
    let villager = Target { class: 4, ..BOAR };
    let tower = Target { class: 52, ..BOAR };
    let priority_3 = Attacker {
        attack_priority: 3,
        ..MANGONEL
    };
    assert_eq!(spares_friends(&priority_3, &villager), Ok(false));
    assert_eq!(spares_friends(&priority_3, &tower), Ok(true));
    let blast = Attacker {
        area_effect: 1.0,
        ..MANGONEL
    };
    assert!(spares_friends(&blast, &villager).is_err());
    for level in [4, 8] {
        let a = Attacker {
            blast_level: level,
            ..blast
        };
        assert_eq!(spares_friends(&a, &villager), Ok(true));
    }
    let melee = Attacker {
        range: 1.0,
        ..blast
    };
    assert_eq!(spares_friends(&melee, &villager), Ok(true));
    assert_eq!(spares_friends(&MANGONEL, &villager), Ok(true));
}

/// The siege unit's attack check (`FUN_140c19190`): never classes 1 and 42 nor in
/// `NoAttack`; standing its ground within its range of its state position less its own
/// diagonal (`powf`, `sqrtf`), a Gaia target's range 1.5 longer; defending within the range
/// plus 5; a Gaia unit only taking owners by sight; never from below state 2, a human
/// player's packed unit (unless any) or while packing; by priority 0 a ram only at siege and
/// buildings, a trebuchet only at buildings; priority 3 only its classes.
#[test]
fn a_siege_unit_attacks_by_its_stance_its_class_and_its_priority() {
    let villager = |x: f32| Target {
        owner: 2,
        class: 4,
        master: 83,
        position: [x, 20.0, 0.0],
        radius: [0.2, 0.2],
        ..BOAR
    };
    let seeker = |stance| Seeker {
        stance,
        state_position: [20.0, 20.0],
        ..IDLE
    };
    let check = |me: &Attacker, s: &Seeker, t: &Target| siege_can_attack(me, s, t, false);
    assert_eq!(check(&MANGONEL, &seeker(0), &villager(40.0)), Ok(true));
    assert_eq!(check(&MANGONEL, &seeker(3), &villager(21.0)), Ok(false));
    for class in [1, 42] {
        let t = Target {
            class,
            ..villager(21.0)
        };
        assert_eq!(check(&MANGONEL, &seeker(0), &t), Ok(false));
    }
    // the diagonal: sqrt(0.36 + 0.36) = 0.8485; a gap of 7.8 - 0.8485 is within 7
    let diagonal = crate::math::fmath::sqrt(0.6f32 * 0.6 + 0.6 * 0.6);
    let x = 20.0 + 0.2 + 7.0 + diagonal;
    assert_eq!(check(&MANGONEL, &seeker(2), &villager(x - 0.01)), Ok(true));
    assert_eq!(check(&MANGONEL, &seeker(2), &villager(x + 0.01)), Ok(false));
    let gaia = Target {
        owner: 0,
        ..villager(x + 1.4)
    };
    assert_eq!(check(&MANGONEL, &seeker(2), &gaia), Ok(true));
    let defend = seeker(1);
    assert_eq!(check(&MANGONEL, &defend, &villager(x + 4.99)), Ok(true));
    assert_eq!(check(&MANGONEL, &defend, &villager(x + 5.01)), Ok(false));
    let patrolling = Seeker {
        order: PATROL,
        ..defend
    };
    assert!(check(&MANGONEL, &patrolling, &villager(30.0)).is_err());
    // Gaia's unit, a low state, a packed unit, a packing one
    let gaia_unit = Attacker {
        player: 0,
        capture: 1,
        ..MANGONEL
    };
    assert_eq!(check(&gaia_unit, &seeker(0), &villager(30.0)), Ok(false));
    let gaia_taker = Attacker {
        capture: 0,
        ..gaia_unit
    };
    assert_eq!(check(&gaia_taker, &seeker(0), &villager(30.0)), Ok(true));
    let low = Attacker {
        state: 1,
        ..MANGONEL
    };
    assert_eq!(check(&low, &seeker(0), &villager(30.0)), Ok(false));
    let packed = Attacker {
        class: 51,
        ..MANGONEL
    };
    assert_eq!(check(&packed, &seeker(0), &villager(30.0)), Ok(false));
    assert_eq!(
        siege_can_attack(&packed, &seeker(0), &villager(30.0), true),
        Ok(true)
    );
    let packing = Attacker {
        top_action: Some(124),
        ..MANGONEL
    };
    assert_eq!(check(&packing, &seeker(0), &villager(30.0)), Ok(false));
    // a ram attacks buildings and siege, a trebuchet buildings only
    let ram = Attacker {
        master: 35,
        ..MANGONEL
    };
    let siege = Target {
        class: 13,
        ..villager(30.0)
    };
    assert_eq!(check(&ram, &seeker(0), &villager(30.0)), Ok(false));
    assert_eq!(check(&ram, &seeker(0), &siege), Ok(true));
    let trebuchet = Attacker {
        class: 54,
        ..MANGONEL
    };
    assert_eq!(check(&trebuchet, &seeker(0), &siege), Ok(false));
    let house = Target {
        class: 3,
        kind: 80,
        ..villager(30.0)
    };
    assert_eq!(check(&trebuchet, &seeker(0), &house), Ok(true));
    // masters 1744 and 1746 on a building: priority 1 or 4 and up only
    let elephant = |attack_priority| Attacker {
        master: 1744,
        attack_priority,
        ..MANGONEL
    };
    let on_building = Seeker {
        target: 900,
        target_kind: Some(80),
        ..seeker(0)
    };
    for (priority, attacks) in [(0, false), (1, true), (2, false), (3, false), (4, true)] {
        assert_eq!(
            check(&elephant(priority), &on_building, &villager(30.0)),
            Ok(attacks),
            "priority {priority}"
        );
    }
    let priority_3 = Attacker {
        attack_priority: 3,
        ..MANGONEL
    };
    assert_eq!(check(&priority_3, &seeker(0), &villager(30.0)), Ok(false));
    assert_eq!(check(&priority_3, &seeker(0), &house), Ok(true));
    // its neutral test spares the trade cart's class no more
    assert!(siege_attacks_neutral(0, 19));
    assert!(!soldier_attacks_neutral(0, 19));
    assert!(!siege_attacks_neutral(0, 4));
}

/// A siege unit's world: its search finds `found`, the attack check lets `allowed` be
/// attacked, the attack command takes, and `reach` answers the reach test.
#[derive(Default)]
struct SiegeSide {
    found: Option<Found>,
    allowed: Option<(i16, [f32; 3])>,
    attacks: Vec<(i32, bool)>,
    reach: bool,
    searches: usize,
}

impl UnitSide for SiegeSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        self.searches += 1;
        Ok(self.found.unwrap_or(Found::NONE))
    }
    fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        self.attacks.push((target, force));
        Ok(Attacked {
            attacks: true,
            posted: Vec::new(),
        })
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Ok(self.allowed)
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn reaches(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
        Ok(self.reach)
    }
}

/// The siege unit's idle check (`FUN_140c206b0`): it caps the last human order time, finds a
/// target with `bestUnitToAttack` and attacks it (forced), the order going (6); finding
/// nothing, a unit in a group gives 5 and one defending walks back. Gaia's that takes no
/// owners by sight does nothing.
#[test]
fn a_siege_unit_attacks_what_its_idle_search_finds() {
    let at = Surroundings {
        id: 500,
        position: [20.0, 20.0, 0.0],
        player: 1,
        class: 13,
        kind: 70,
        los: 9.0,
        weapon_range: 7.0,
        state: 2,
        ..Surroundings::default()
    };
    let mut u = UnitAi {
        current_order: MOVE_ORDER,
        ..idle(SIEGE_AI)
    };
    let mut side = SiegeSide {
        found: Some(Found {
            id: 77,
            ..Found::NONE
        }),
        allowed: Some((4, [25.0, 20.0, 0.0])),
        ..SiegeSide::default()
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.process_idle(&at, false, &mut rng(), &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
    assert_eq!(side.attacks, vec![(77, true)]);
    assert_eq!(
        (u.current_action, u.current_target, u.current_order),
        (ATTACK_ACTION, 77, -1)
    );
    // nothing found: in a group 5 and no walk; defending without a group walks back
    let grouped = Surroundings {
        group: Some(GroupState::default()),
        ..at.clone()
    };
    let mut u = UnitAi {
        stance: 1,
        state_position: [10.0, 20.0],
        ..idle(SIEGE_AI)
    };
    let mut side = SiegeSide::default();
    let mut fx = Vec::new();
    assert_eq!(
        u.process_idle(&grouped, false, &mut rng(), &mut side, &mut fx),
        Ok(false)
    );
    assert_eq!(fx.len(), 1);
    let mut fx = Vec::new();
    assert_eq!(
        u.process_idle(&at, false, &mut rng(), &mut side, &mut fx),
        Ok(false)
    );
    assert!(matches!(fx.last(), Some(Effect::Move(_))));
    // Gaia's that does not take owners by sight
    let gaia = Surroundings {
        player: 0,
        capture: 1,
        ..at.clone()
    };
    let mut side = SiegeSide::default();
    assert_eq!(
        idle(SIEGE_AI).process_idle(&gaia, false, &mut rng(), &mut side, &mut Vec::new()),
        Ok(false)
    );
    assert_eq!(side.searches, 0);
}

/// With retargeting off and its current target held, a top target the siege unit reaches
/// within its range ends the check (5); not reached, retargeting comes back on.
#[test]
fn a_siege_unit_with_retargeting_off_keeps_a_reachable_top_target() {
    let at = Surroundings {
        id: 500,
        position: [20.0, 20.0, 0.0],
        player: 1,
        class: 13,
        kind: 70,
        weapon_range: 7.0,
        state: 2,
        target: Some(Seen::default()),
        top_target: Some(TopTarget {
            id: 88,
            kind: 70,
            position: [24.0, 20.0, 0.0],
            radius: [0.2, 0.2],
        }),
        ..Surroundings::default()
    };
    let mut u = UnitAi {
        retarget_disabled: 1,
        current_target: 88,
        ..idle(SIEGE_AI)
    };
    let mut side = SiegeSide {
        reach: true,
        ..SiegeSide::default()
    };
    assert_eq!(
        u.process_idle(&at, false, &mut rng(), &mut side, &mut Vec::new()),
        Ok(false)
    );
    assert_eq!((u.retarget_disabled, side.attacks.len()), (1, 0));
    let mut side = SiegeSide {
        allowed: Some((4, [24.0, 20.0, 0.0])),
        ..SiegeSide::default()
    };
    assert_eq!(
        u.process_idle(&at, false, &mut rng(), &mut side, &mut Vec::new()),
        Ok(true)
    );
    assert_eq!(
        (u.retarget_disabled, side.attacks.clone()),
        (0, vec![(88, true)])
    );
}

/// The siege unit's handler (`FUN_140c287e0`): a target under its least range makes it back
/// up (not modelled); a human player's executed action and damage are the base handler's; a
/// computer player's executed attack draws whether it keeps its distance.
#[test]
fn a_siege_unit_hands_its_notifies_to_the_base_handler() {
    let at = Surroundings {
        id: 500,
        weapon_range: 7.0,
        player: 1,
        ..Surroundings::default()
    };
    let mut u = idle(SIEGE_AI);
    let mut side = SiegeSide::default();
    let under = Notify::own(500, 0x1fe, [77, 0, 0]);
    assert!(
        u.handle(under, 0, &at, false, &mut rng(), &mut side, &mut Vec::new())
            .is_err()
    );
    let executed = Notify::own(500, ACTION_EXECUTED, [600, 77, 0]);
    assert_eq!(
        u.handle(
            executed,
            0,
            &at,
            false,
            &mut rng(),
            &mut side,
            &mut Vec::new()
        ),
        Ok(GO_ON)
    );
    // a computer player's, off the attack action: one draw, then 2
    let mut r = rng();
    assert_eq!(
        u.handle(executed, 0, &at, true, &mut r, &mut side, &mut Vec::new()),
        Ok(GO_ON)
    );
    assert_eq!(r.draws, 1);
    // a building target: no draw, the base handler's
    let mut on_house = UnitAi {
        target_type: 3,
        ..idle(SIEGE_AI)
    };
    let mut r = rng();
    assert_eq!(
        on_house.handle(executed, 0, &at, true, &mut r, &mut side, &mut Vec::new()),
        Ok(GO_ON)
    );
    assert_eq!(r.draws, 0);
    // its run: a top target or class 54 keeps it
    let pinned = Surroundings {
        top_target: Some(TopTarget::default()),
        ..at.clone()
    };
    let mut fx = Vec::new();
    assert_eq!(
        u.run_away(true, &pinned, false, &mut rng(), &mut side, &mut fx),
        Ok(false)
    );
    assert_eq!(fx, Vec::new());
}

/// The priest's slot 73 (`FUN_140c21f30`): the base's retarget check (a used timer redrawn),
/// but on the order on a target and for a computer player's priest none.
#[test]
fn a_priest_retargets_but_on_an_order_on_a_target() {
    let at = Surroundings {
        player: 1,
        ..Surroundings::default()
    };
    let priest = |order| UnitAi {
        current_order: order,
        retarget_timer: 900,
        last_retarget_time: 0,
        adjusted_idle_timeout: 9000,
        look_around_timeout: 9000,
        ..idle(PRIEST_AI)
    };
    for (order, computer, draws) in [(-1, false, 1), (ORDER_ON_TARGET, false, 0), (-1, true, 0)] {
        let mut u = priest(order);
        let mut r = rng();
        let _ = u.update_tail(
            Retarget::Priest,
            10_000,
            computer,
            false,
            None,
            &at,
            &mut r,
            &mut Recorder::default(),
            &mut Vec::new(),
        );
        assert_eq!(r.draws, draws, "order {order}, computer {computer}");
    }
    assert!(matches!(retarget_kind(PRIEST_AI), Ok(Retarget::Priest)));
}

/// A side for the full update's tail: the unit's top action's target now, the objects'
/// flags and states.
#[derive(Default)]
struct TailSide {
    top: Option<i32>,
    flags: Vec<(i32, TargetFlags)>,
    states: Vec<(i32, u8)>,
}

impl UnitSide for TailSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Err(GenieError::unsupported("no attack here"))
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        Ok(())
    }
    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn top_target_now(&mut self) -> Result<Option<i32>, GenieError> {
        Ok(self.top)
    }
    fn target_flags(&mut self, id: i32) -> Result<Option<TargetFlags>, GenieError> {
        Ok(self.flags.iter().find(|f| f.0 == id).map(|f| f.1))
    }
    fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
        Ok(self
            .states
            .iter()
            .find(|s| s.0 == id)
            .map(|&(id, state)| UnitSeen {
                id,
                state,
                ..UnitSeen::default()
            }))
    }
}

/// A target the tail lets be: a player's, targetable and attackable, outside a pasture.
const PLAIN: TargetFlags = TargetFlags {
    owner: 2,
    kind: 70,
    capture: 0,
    pasture: -1,
    untargetable: false,
    attackable: true,
};

fn on_target(action: i32) -> UnitAi {
    UnitAi {
        current_action: action,
        current_target: 500,
        target_type: 6,
        target_point: [10.0, 10.0, 0.0],
        ..idle(5)
    }
}

fn tail(u: &mut UnitAi, player: usize, flags: TargetFlags, fx: &mut Vec<Effect>) {
    let at = Surroundings {
        player,
        ..Surroundings::default()
    };
    let mut side = TailSide {
        top: Some(500),
        flags: vec![(500, flags)],
        ..TailSide::default()
    };
    u.top_target_tail(&at, &mut side, fx).unwrap();
}

#[test]
fn the_tail_stops_a_gaia_unit_on_a_gaia_object_others_can_take_again() {
    // 0x140c68519: a Gaia unit, a Gaia target of type 70 or more, its +0x924 2: the target
    // goes (FUN_140c5e1e0) and the action stops (slot 0xb0 with 1)
    let again = TargetFlags {
        owner: 0,
        capture: 2,
        ..PLAIN
    };
    let mut u = on_target(ATTACK_ACTION);
    let mut fx = Vec::new();
    tail(&mut u, 0, again, &mut fx);
    assert_eq!(fx, [Effect::Stop]);
    assert_eq!((u.current_target, u.last_target), (-1, 500));
    assert_eq!((u.current_action, u.last_action), (-1, ATTACK_ACTION));
    // a player's unit, a player's target, another capture flag or a type below 70 stays
    for (player, flags) in [
        (1, again),
        (0, TargetFlags { owner: 1, ..again }),
        (
            0,
            TargetFlags {
                capture: 1,
                ..again
            },
        ),
        (0, TargetFlags { kind: 60, ..again }),
    ] {
        let mut u = on_target(ATTACK_ACTION);
        let mut fx = Vec::new();
        tail(&mut u, player, flags, &mut fx);
        assert_eq!(fx, Vec::<Effect>::new(), "{player} {flags:?}");
        assert_eq!(u.current_target, 500);
    }
}

#[test]
fn the_tail_stops_work_on_a_target_without_its_targeting_flag() {
    // 0x140c685cf: outside a pasture, a target that cannot be targeted (+0x689 0) stops any
    // action but the trade
    let untargetable = TargetFlags {
        untargetable: true,
        ..PLAIN
    };
    for action in [GATHER_ACTION, ATTACK_ACTION, -1] {
        let mut u = on_target(action);
        let mut fx = Vec::new();
        tail(&mut u, 1, untargetable, &mut fx);
        assert_eq!(fx, [Effect::Stop], "{action}");
        assert_eq!(u.current_target, -1);
    }
    let mut u = on_target(TRADE_ACTION);
    let mut fx = Vec::new();
    tail(&mut u, 1, untargetable, &mut fx);
    assert_eq!(fx, Vec::<Effect>::new());
    // one in a pasture stays
    let mut u = on_target(GATHER_ACTION);
    tail(
        &mut u,
        1,
        TargetFlags {
            pasture: 77,
            ..untargetable
        },
        &mut fx,
    );
    assert_eq!(fx, Vec::<Effect>::new());
    // one that cannot be attacked (+0x68a 0) stops only the attack
    let unattackable = TargetFlags {
        attackable: false,
        ..PLAIN
    };
    let mut u = on_target(GATHER_ACTION);
    tail(&mut u, 1, unattackable, &mut fx);
    assert_eq!(fx, Vec::<Effect>::new());
    let mut u = on_target(ATTACK_ACTION);
    tail(&mut u, 1, unattackable, &mut fx);
    assert_eq!(fx, [Effect::Stop]);
    // both: the second test reads the action value the first stop left (−1), so an
    // untargetable target stops it again
    let both = TargetFlags {
        owner: 0,
        capture: 2,
        untargetable: true,
        ..PLAIN
    };
    let mut u = on_target(ATTACK_ACTION);
    let mut fx = Vec::new();
    tail(&mut u, 0, both, &mut fx);
    assert_eq!(fx, [Effect::Stop, Effect::Stop]);
    assert_eq!((u.current_action, u.last_action), (-1, -1));
    let unattackable_again = TargetFlags {
        untargetable: false,
        attackable: false,
        ..both
    };
    let mut u = on_target(ATTACK_ACTION);
    let mut fx = Vec::new();
    tail(&mut u, 0, unattackable_again, &mut fx);
    assert_eq!(fx, [Effect::Stop]);
}

#[test]
fn the_tail_reads_the_top_target_the_update_s_commands_left() {
    let untargetable = TargetFlags {
        untargetable: true,
        ..PLAIN
    };
    let at = Surroundings {
        player: 1,
        ..Surroundings::default()
    };
    let mut side = TailSide {
        top: Some(500),
        flags: vec![(500, untargetable), (600, untargetable)],
        ..TailSide::default()
    };
    // a move to a point, a stop or an explore left no object to test
    for pending in [
        Effect::Stop,
        Effect::Explore([1.0, 2.0]),
        Effect::Move(AiMove {
            point: [1.0, 2.0, 0.0],
            range: 0.0,
            force: true,
            walk: false,
        }),
    ] {
        let mut u = on_target(GATHER_ACTION);
        let mut fx = vec![pending];
        u.top_target_tail(&at, &mut side, &mut fx).unwrap();
        assert_eq!(fx, [pending]);
    }
    // a move to an object holds it
    let mut u = on_target(MOVE_ACTION);
    let to = Effect::MoveToObject {
        target: 600,
        state: 2,
        range: 2.0,
    };
    let mut fx = vec![to];
    u.top_target_tail(&at, &mut side, &mut fx).unwrap();
    assert_eq!(fx, [to, Effect::Stop]);
    // after a hunt the caller gives once the update ends, the stop would reach the unit
    // first
    let mut u = on_target(HUNT_ACTION);
    let mut fx = vec![Effect::HuntObject(600)];
    assert!(u.top_target_tail(&at, &mut side, &mut fx).is_err());
    // no top action, or one the world no longer holds: nothing
    let mut u = on_target(GATHER_ACTION);
    let mut fx = Vec::new();
    u.top_target_tail(
        &at,
        &mut TailSide {
            top: Some(700),
            ..TailSide::default()
        },
        &mut fx,
    )
    .unwrap();
    u.top_target_tail(&at, &mut TailSide::default(), &mut fx)
        .unwrap();
    assert_eq!(fx, Vec::<Effect>::new());
}

#[test]
fn queued_builds_on_started_or_gone_foundations_leave_the_queue() {
    // 0x140c686e0: off the build order, each queued build order (the last first) on a
    // foundation the world does not hold or holds out of state 0 goes; other orders stay
    let order = |order: i32, target: i32| QueuedOrder {
        order,
        target,
        ready: true,
        ..QueuedOrder::default()
    };
    let queue = vec![
        order(BUILD_ORDER, 1),
        order(BUILD_ORDER, 2),
        order(MOVE_ORDER, 3),
        order(BUILD_ORDER, 4),
        order(BUILD_ORDER, 5),
    ];
    let mut side = TailSide {
        states: vec![(1, 0), (2, 2), (3, 2), (5, 0)],
        ..TailSide::default()
    };
    let mut u = UnitAi {
        orders: queue.clone(),
        ..idle(8)
    };
    u.drop_started_builds(&mut side).unwrap();
    let left: Vec<i32> = u.orders.iter().map(|o| o.target).collect();
    assert_eq!(left, [1, 3, 5]);
    // on the build order the queue stays
    let mut u = UnitAi {
        current_order: BUILD_ORDER,
        orders: queue,
        ..idle(8)
    };
    u.drop_started_builds(&mut side).unwrap();
    assert_eq!(u.orders.len(), 5);
}

#[test]
fn a_unit_standing_its_ground_that_strayed_with_an_action_stops_and_walks_back() {
    // slot 73's stand ground leash (0x140c57bc4): the squared gap to the state position
    // (4 tiles: 16) against the range plus 0.25
    let at = Surroundings {
        position: [14.0, 10.0, 0.0],
        weapon_range: 15.0,
        player: 1,
        ..Surroundings::default()
    };
    let standing = |action| UnitAi {
        stance: 2,
        state_position: [10.0, 10.0],
        current_action: action,
        current_target: 500,
        retarget_timer: 100_000,
        ..idle(13)
    };
    let mut u = standing(ATTACK_ACTION);
    let mut fx = Vec::new();
    assert_eq!(
        u.retarget_check(0, false, &at, &mut rng(), &mut finds_nothing(), &mut fx),
        Ok(false)
    );
    assert_eq!(fx[0], Effect::Stop);
    assert_eq!(
        fx[1],
        Effect::Move(AiMove {
            point: [10.0, 10.0, 0.0],
            range: WALK_BACK,
            force: true,
            walk: false,
        })
    );
    assert_eq!((u.current_target, u.current_action), (-1, MOVE_ACTION));
    // within the range plus 0.25, without an action value, moving aside, on a patrol order
    // or in a group that patrols: it stays
    let near = Surroundings {
        weapon_range: 15.75,
        ..at.clone()
    };
    let patrolling = Surroundings {
        group: Some(GroupState {
            patrol: true,
            ..GroupState::default()
        }),
        ..at.clone()
    };
    for (mut u, at) in [
        (standing(ATTACK_ACTION), &near),
        (standing(-1), &at),
        (standing(MOVE_ASIDE_ACTION), &at),
        (standing(GROUP_ASIDE_ACTION), &at),
        (
            UnitAi {
                current_order: PATROL,
                ..standing(ATTACK_ACTION)
            },
            &at,
        ),
        (standing(ATTACK_ACTION), &patrolling),
    ] {
        let mut fx = Vec::new();
        let _ = u
            .retarget_check(0, false, at, &mut rng(), &mut finds_nothing(), &mut fx)
            .unwrap();
        assert_eq!(fx, Vec::<Effect>::new(), "{:?}", u.current_action);
    }
    // a gap that is not a number stays (COMISS, JBE)
    let mut u = UnitAi {
        state_position: [f32::NAN, 10.0],
        ..standing(ATTACK_ACTION)
    };
    let mut fx = Vec::new();
    let _ = u
        .retarget_check(0, false, &at, &mut rng(), &mut finds_nothing(), &mut fx)
        .unwrap();
    assert_eq!(fx, Vec::<Effect>::new());
}

/// `FUN_14087e7c0` and `FUN_140f95e10`: the least distance to the lines through a path's
/// consecutive points (the lines, not the segments); a path of one point answers the largest
/// f32, an empty one is unreachable; a NaN distance is kept until the next one.
#[test]
fn a_patrol_s_distance_is_the_least_to_the_lines_through_its_points() {
    assert_eq!(
        line_distance([3.0, 0.0, 4.0], [0.0; 3], [10.0, 0.0, 0.0]),
        4.0
    );
    // beyond the segment's end the line still counts
    assert_eq!(
        line_distance([30.0, 0.0, 4.0], [0.0; 3], [10.0, 0.0, 0.0]),
        4.0
    );
    let path = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 0.0, 10.0]];
    assert_eq!(patrol_distance(&path, [9.0, 0.0, 4.0]), Ok(1.0));
    assert_eq!(
        patrol_distance(&path[..1], [9.0, 0.0, 4.0]),
        Ok(PATROL_FARTHEST)
    );
    assert!(patrol_distance(&[], [9.0, 0.0, 4.0]).is_err());
    // two equal points make a NaN, which the next line replaces
    let doubled = [[0.0; 3], [0.0; 3], [10.0, 0.0, 0.0]];
    assert_eq!(patrol_distance(&doubled, [3.0, 0.0, 4.0]), Ok(4.0));
    assert!(
        patrol_distance(&doubled[..2], [3.0, 0.0, 4.0])
            .unwrap()
            .is_nan()
    );
}

/// processMisc's leash with patrol information (`+0x1c0`): from where the unit last moved to
/// (slot 0x520) to the saved patrol's path (`FUN_140f95e10`). Beyond stand ground's 0.5 or
/// the defend stance's 5 the unit stops, forgets its target and walks its patrol on, its
/// order the patrol's (10); within it stand ground goes on to the retarget check (its
/// state position's leash not read), the defend stance ends slot 73 (9) before it.
#[test]
fn a_unit_with_patrol_information_leashes_to_its_patrol_s_path() {
    let path = PatrolPath {
        path: crate::pathing::tile_path::Path {
            points: vec![[0.0, 0.0, 10.0], [20.0, 0.0, 10.0]],
            length: 20.0,
        },
        flags: 3,
    };
    // where the unit last moved to: on the path's line, 1 or 6 off it
    let at = |y: f32| Surroundings {
        position: [14.0, y, 0.0],
        moved_at: [14.0, 0.0, y],
        weapon_range: 15.0,
        player: 1,
        ..Surroundings::default()
    };
    let unit = |stance| UnitAi {
        stance,
        state_position: [10.0, 30.0],
        current_action: ATTACK_ACTION,
        current_target: 500,
        retarget_timer: 0,
        last_retarget_time: 0,
        patrol_path: Some(path.clone()),
        ..idle(13)
    };
    let run = |u: &mut UnitAi, at: &Surroundings| {
        let mut fx = Vec::new();
        let r = u.retarget_check(1000, false, at, &mut rng(), &mut finds_nothing(), &mut fx);
        (r, fx)
    };
    // stand ground within 0.5: no leash, on to the retarget check (the timer runs out)
    let mut u = unit(2);
    let (r, fx) = run(&mut u, &at(10.0));
    assert_eq!((r, fx), (Ok(true), vec![]));
    assert_eq!(u.last_retarget_time, 1000);
    // beyond it: stop, the target forgotten, the patrol walked on (10)
    let mut u = unit(2);
    let (r, fx) = run(&mut u, &at(11.0));
    assert_eq!(
        (r, fx),
        (Ok(false), vec![Effect::Stop, Effect::RestorePatrol])
    );
    assert_eq!(
        (u.current_target, u.current_order, u.secondary_timer),
        (-1, PATROL, 750)
    );
    // the defend stance within 5 ends slot 73 before the retarget check (9)
    let mut u = unit(1);
    let (r, fx) = run(&mut u, &at(11.0));
    assert_eq!((r, fx), (Ok(false), vec![]));
    assert_eq!(u.last_retarget_time, 0);
    // beyond 5 it walks the patrol on
    let mut u = unit(1);
    let (r, fx) = run(&mut u, &at(16.0));
    assert_eq!(
        (r, fx),
        (Ok(false), vec![Effect::Stop, Effect::RestorePatrol])
    );
}

#[test]
fn a_busy_villager_on_the_attack_order_runs_its_full_update() {
    // records (6)/426406723's villagers attacking (order 700, action 600, a target), busy: the
    // update runs whole (the civilian's processMisc, the idle part keeping the timer of a
    // player's unit with an action at 0)
    let mut u = UnitAi {
        current_order: ATTACK_ORDER,
        current_action: ATTACK_ACTION,
        current_target: 18842,
        target_type: 10,
        idle_timer: 400,
        ..idle(8)
    };
    let at = Surroundings {
        busy: true,
        player: 3,
        class: CLASS_VILLAGER,
        state: 2,
        target: Some(Seen {
            state: 2,
            kind: 70,
            owner: 0,
            class: 10,
            ..Seen::default()
        }),
        ..Surroundings::default()
    };
    let fx = u
        .update(10, 1000, false, &at, &mut rng(), &mut TailSide::default())
        .unwrap();
    assert_eq!(fx, Vec::<Effect>::new());
    assert_eq!(u.idle_timer, 0);
    assert_eq!(
        (u.current_order, u.current_action, u.current_target),
        (ATTACK_ORDER, ATTACK_ACTION, 18842)
    );
}

/// A side for the order slots: the objects the world holds (their class, place, state and
/// gap), the unit's attack commands and other commands, which it takes unless `refuses`, the
/// reach test's answer, the soldier's group stops and the waits for foundations.
#[derive(Default)]
struct OrderSide {
    world: Vec<UnitSeen>,
    attacks: Vec<(i32, bool)>,
    commands: Vec<UnitCommand>,
    refuses: bool,
    reach: bool,
    stops: usize,
    group_stops: usize,
    waits: Vec<i32>,
    /// What the resource searches find, and the searches asked.
    resource: Option<Gatherable>,
    searches: Vec<ResourceSearch>,
    /// The classes of the players' masters: ((player, master), class).
    classes: Vec<((i32, i32), i16)>,
}

impl OrderSide {
    /// A world of objects `(id, class)` in state 2 at (10, 20, 0), a gap of 1 from the unit.
    fn holding(objects: &[(i32, i16)]) -> Self {
        Self {
            world: objects
                .iter()
                .map(|&(id, class)| UnitSeen {
                    id,
                    class,
                    state: 2,
                    gap_squared: 1.0,
                    position: [10.0, 20.0, 0.0],
                    ..UnitSeen::default()
                })
                .collect(),
            ..Self::default()
        }
    }
}

impl UnitSide for OrderSide {
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
    }
    fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        self.attacks.push((target, force));
        Ok(Attacked {
            attacks: !self.refuses,
            posted: Vec::new(),
        })
    }
    fn set_under_attack(&mut self, _: bool) {}
    fn stop(&mut self) -> Result<(), GenieError> {
        self.stops += 1;
        Ok(())
    }
    fn may_attack(
        &mut self,
        target: i32,
        _: &Seeker,
    ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Ok(self.seen_unit(target)?.map(|t| (t.class, t.position)))
    }
    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported("no search here"))
    }
    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("no world here"))
    }
    fn reaches(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
        Ok(self.reach)
    }
    fn can_stand(&mut self, _: [f32; 2]) -> Result<bool, GenieError> {
        Ok(true)
    }
    fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
        Ok(self.world.iter().find(|u| u.id == id).copied())
    }
    fn wait_solidify(&mut self, target: i32) -> Result<bool, GenieError> {
        self.waits.push(target);
        Ok(!self.refuses)
    }
    fn command(&mut self, command: UnitCommand) -> Result<bool, GenieError> {
        self.commands.push(command);
        Ok(!self.refuses)
    }
    fn group_attack_stop(&mut self) -> Result<(), GenieError> {
        self.group_stops += 1;
        Ok(())
    }
    fn closest_resource(
        &mut self,
        search: ResourceSearch,
    ) -> Result<Option<Gatherable>, GenieError> {
        self.searches.push(search);
        Ok(self.resource)
    }
    fn master_class(&mut self, player: i32, master: i32) -> Result<Option<i16>, GenieError> {
        Ok(self
            .classes
            .iter()
            .find(|c| c.0 == (player, master))
            .map(|c| c.1))
    }
}

/// An order of value `order` on `target` at (12.7, 30.2, 0.5) within 3, priority 40, ready.
fn queued(order: i32, target: i32) -> QueuedOrder {
    QueuedOrder {
        issuer: 1,
        order,
        priority: 40,
        target,
        target_owner: -1,
        point: [12.7, 30.2, 0.5],
        range: 3.0,
        shown: [12.5, 30.5],
        ready: true,
        target_type: -1,
    }
}

/// Slot 0x230 on order `o` inside the update: the unit of class `class` at (5, 6, 0.25), its
/// master 4, player 1; the result, the effects and the draws.
fn run_order(
    u: &mut UnitAi,
    o: QueuedOrder,
    class: i16,
    side: &mut OrderSide,
) -> (Result<u32, GenieError>, Vec<Effect>, u32) {
    let at = Surroundings {
        id: 9,
        position: [5.0, 6.0, 0.25],
        map: [100, 100],
        class,
        master: 4,
        player: 1,
        state: 2,
        ..Surroundings::default()
    };
    let mut fx = Vec::new();
    let mut r = rng();
    let res = u.process_order(&o, Some((&at, side, false)), None, &mut r, &mut fx);
    (res, fx, r.draws)
}

#[test]
fn an_order_not_ready_stays_and_patrol_or_unnamed_orders_take_nothing() {
    let mut u = idle(13);
    let mut side = OrderSide::default();
    let waiting = QueuedOrder {
        ready: false,
        ..queued(MOVE_ORDER, -1)
    };
    assert_eq!(run_order(&mut u, waiting, 6, &mut side).0, Ok(0xe));
    // "I don't know what to do with patrol orders"; an order no case names
    for order in [PATROL, 0x2c4, 0x2cb, 0x2d5, 0x323] {
        let (r, fx, _) = run_order(&mut u, queued(order, -1), 6, &mut side);
        assert_eq!((r, fx), (Ok(0), Vec::new()), "{order:#x}");
    }
    assert_eq!(u, idle(13));
}

#[test]
fn the_attack_order_attacks_by_the_unit_ai_s_type_and_becomes_the_ai_s() {
    // a soldier: the base's AttackObject through its slot 0xa0 (the side's test), then a
    // member of a group stops moving (FUN_140c17550)
    let mut u = idle(SOLDIER_AI);
    let mut side = OrderSide::holding(&[(70, 6)]);
    let (r, fx, _) = run_order(&mut u, queued(ATTACK_ORDER, 70), 6, &mut side);
    assert_eq!((r, fx), (Ok(1), Vec::new()));
    assert_eq!(side.attacks, [(70, true)]);
    assert_eq!(side.group_stops, 0);
    assert_eq!(
        (u.current_order, u.order_priority, u.current_action),
        (ATTACK_ORDER, 40, ATTACK_ACTION)
    );
    assert_eq!(
        (u.current_target, u.target_type, u.target_point),
        (70, 6, [10.0, 20.0, 0.0])
    );
    let mut u = idle(SOLDIER_AI);
    let mut side = OrderSide::holding(&[(70, 6)]);
    let o = queued(ATTACK_ORDER, 70);
    let at = Surroundings {
        class: 6,
        group: Some(GroupState::default()),
        ..Surroundings::default()
    };
    let r = u.process_order(
        &o,
        Some((&at, &mut side, false)),
        None,
        &mut rng(),
        &mut Vec::new(),
    );
    assert_eq!((r, side.group_stops), (Ok(1), 1));
    // a villager hunts an animal (class 9) and attacks anything else it may (class 6)
    let mut u = idle(CIVILIAN);
    let mut side = OrderSide::holding(&[(71, 9), (72, 6)]);
    let (r, fx, _) = run_order(&mut u, queued(ATTACK_ORDER, 71), 4, &mut side);
    assert_eq!((r, fx), (Ok(1), vec![Effect::HuntObject(71)]));
    assert_eq!(
        (u.current_order, u.current_action),
        (ATTACK_ORDER, HUNT_ACTION)
    );
    let mut u = idle(CIVILIAN);
    let (r, _, _) = run_order(&mut u, queued(ATTACK_ORDER, 72), 4, &mut side);
    assert_eq!((r, u.current_action), (Ok(1), ATTACK_ACTION));
    // a unit of class 18: the tail forgets the target (its class and point stay) and ends
    // the order, the order's priority kept
    let mut u = idle(BOAR_AI);
    let (r, _, _) = run_order(&mut u, queued(ATTACK_ORDER, 72), CLASS_18, &mut side);
    assert_eq!(r, Ok(1));
    assert_eq!(
        (
            u.current_order,
            u.order_priority,
            u.current_target,
            u.target_type
        ),
        (-1, 40, -1, 6)
    );
    // a command the unit refuses, or a target the world does not hold: not taken
    let mut u = idle(BOAR_AI);
    side.refuses = true;
    assert_eq!(
        run_order(&mut u, queued(ATTACK_ORDER, 72), 10, &mut side).0,
        Ok(0)
    );
    assert_eq!(
        run_order(&mut u, queued(ATTACK_ORDER, 99), 10, &mut side).0,
        Ok(0)
    );
    assert_eq!((u.current_order, u.current_action), (-1, -1));
}

#[test]
fn the_defend_order_keeps_near_its_target_or_its_point() {
    let mut side = OrderSide::holding(&[(70, 6)]);
    let mut u = UnitAi {
        current_action: MOVE_ACTION,
        ..idle(SOLDIER_AI)
    };
    let (r, fx, _) = run_order(&mut u, queued(DEFEND_ORDER, 70), 6, &mut side);
    assert_eq!((r, fx), (Ok(1), Vec::new()));
    assert_eq!(
        (u.current_order, u.current_action, u.last_action),
        (DEFEND_ORDER, DEFEND_ACTION, MOVE_ACTION)
    );
    assert_eq!(
        (u.current_target, u.target_type, u.target_point),
        (70, 6, [10.0, 20.0, 0.0])
    );
    assert_eq!((u.desired_distance, u.defend_target), (3.0, 70));
    // without a target: the point at the unit's height, no defend target
    let mut u = idle(SOLDIER_AI);
    assert_eq!(
        run_order(&mut u, queued(DEFEND_ORDER, -1), 6, &mut side).0,
        Ok(1)
    );
    assert_eq!(
        (u.current_target, u.target_point, u.defend_target),
        (-1, [12.7, 30.2, 0.25], -1)
    );
    // a target the world does not hold: not taken
    let mut u = idle(SOLDIER_AI);
    assert_eq!(
        run_order(&mut u, queued(DEFEND_ORDER, 99), 6, &mut side).0,
        Ok(0)
    );
    // master 128 takes no defend order
    let at = Surroundings {
        master: DEFENDLESS_MASTER,
        ..Surroundings::default()
    };
    let o = queued(DEFEND_ORDER, -1);
    let r = u.process_order(
        &o,
        Some((&at, &mut side, false)),
        None,
        &mut rng(),
        &mut Vec::new(),
    );
    assert_eq!(r, Ok(0));
    assert_eq!(u.current_action, -1);
}

#[test]
fn the_heal_conversion_and_repair_orders_give_the_unit_s_commands() {
    for (order, action, command) in [
        (ai_kinds::ORDER_HEAL, HEAL_ACTION, UnitCommand::Heal(70)),
        (
            ai_kinds::ORDER_CONVERT,
            CONVERT_ACTION,
            UnitCommand::Convert(70),
        ),
        (REPAIR_ORDER, REPAIR_ACTION, UnitCommand::Repair(70)),
    ] {
        let mut side = OrderSide::holding(&[(70, 3)]);
        let mut u = UnitAi {
            current_target: 5,
            target_type: 9,
            ..idle(PRIEST_AI)
        };
        let (r, fx, _) = run_order(&mut u, queued(order, 70), 18, &mut side);
        assert_eq!((r, fx), (Ok(1), Vec::new()));
        assert_eq!(side.commands, [command]);
        assert_eq!((u.current_order, u.current_action), (order, action));
        assert_eq!(
            (u.current_target, u.target_type, u.target_point),
            (70, 3, [10.0, 20.0, 0.0])
        );
        assert_eq!((u.last_target, u.last_target_type), (5, 9));
        // refused: nothing changes
        let mut u = idle(PRIEST_AI);
        side.refuses = true;
        assert_eq!(run_order(&mut u, queued(order, 70), 18, &mut side).0, Ok(0));
        assert_eq!(u, idle(PRIEST_AI));
    }
    // the unit's commands the slice models on no unit
    let mut u = idle(PRIEST_AI);
    let o = queued(ai_kinds::ORDER_HEAL, 70);
    let at = Surroundings::default();
    let mut tail = TailSide::default();
    assert!(
        u.process_order(
            &o,
            Some((&at, &mut tail, false)),
            None,
            &mut rng(),
            &mut Vec::new()
        )
        .is_err()
    );
}

#[test]
fn the_explore_orders_explore_their_point_cut_to_tiles() {
    let mut side = OrderSide::default();
    for (order, action, last) in [
        (ai_kinds::ORDER_EXPLORE, EXPLORE_ACTION, -1),
        (ai_kinds::ORDER_SCOUT, SCOUT_ACTION, EXPLORE_ACTION),
        (AUTO_SCOUT_ORDER, AUTO_SCOUT_ACTION, EXPLORE_ACTION),
    ] {
        let mut u = idle(SOLDIER_AI);
        let (r, fx, _) = run_order(&mut u, queued(order, -1), 12, &mut side);
        assert_eq!((r, fx), (Ok(1), vec![Effect::Explore([12.0, 30.0])]));
        assert_eq!(
            (u.current_order, u.current_action, u.last_action),
            (order, action, last)
        );
        assert_eq!(u.target_point, [12.0, 30.0, EXPLORE_Z]);
    }
    // a point that is not a number cuts to i32::MIN (CVTTSS2SI)
    let mut u = idle(SOLDIER_AI);
    let o = QueuedOrder {
        point: [f32::NAN, 1.5, 0.0],
        ..queued(ai_kinds::ORDER_EXPLORE, -1)
    };
    let (_, fx, _) = run_order(&mut u, o, 12, &mut side);
    assert_eq!(fx, vec![Effect::Explore([i32::MIN as f32, 1.0])]);
}

#[test]
fn the_stop_order_stops_and_ends_the_order() {
    let mut side = OrderSide::default();
    let mut u = UnitAi {
        current_order: MOVE_ORDER,
        order_priority: 100,
        current_action: MOVE_ACTION,
        current_target: 70,
        ..idle(SOLDIER_AI)
    };
    let (r, fx, _) = run_order(&mut u, queued(STOP_ORDER, -1), 6, &mut side);
    assert_eq!((r, fx), (Ok(1), vec![Effect::Stop]));
    assert_eq!(
        (
            u.current_order,
            u.order_priority,
            u.current_action,
            u.current_target
        ),
        (-1, -1, -1, -1)
    );
    assert_eq!((u.last_order, u.last_target), (MOVE_ORDER, 70));
    assert_eq!(u.state_position, [5.0, 6.0]);
}

#[test]
fn the_run_away_order_runs_from_the_unit_s_attackers() {
    // no attackers: a random direction (two draws), by twice the speed (0 here: the unit's
    // own place), through evasiveMoveTo
    let mut side = OrderSide::default();
    let mut u = idle(SOLDIER_AI);
    let (r, fx, draws) = run_order(&mut u, queued(RUN_AWAY_ORDER, -1), 6, &mut side);
    assert_eq!(r, Ok(1));
    assert_eq!(draws, 2);
    assert_eq!(
        fx,
        vec![Effect::Move(AiMove {
            point: [5.0, 6.0, 0.25],
            range: 0.0,
            force: true,
            walk: false,
        })]
    );
    assert_eq!(
        (u.current_order, u.current_action),
        (RUN_AWAY_ORDER, MOVE_ACTION)
    );
    // standing its ground, a player's unit stays: not taken
    let mut u = UnitAi {
        stance: 2,
        ..idle(SOLDIER_AI)
    };
    assert_eq!(
        run_order(&mut u, queued(RUN_AWAY_ORDER, -1), 6, &mut side).0,
        Ok(0)
    );
}

#[test]
fn the_gather_and_hunt_orders_work_an_object_the_world_holds() {
    let mut side = OrderSide::holding(&[(70, 15)]);
    let mut u = idle(CIVILIAN);
    let gather = ai_kinds::ORDER_GATHER;
    let (r, fx, _) = run_order(&mut u, queued(gather, 70), 4, &mut side);
    assert_eq!((r, fx), (Ok(1), vec![Effect::GatherObject(70)]));
    assert_eq!(
        (
            u.current_order,
            u.current_action,
            u.current_target,
            u.target_type
        ),
        (gather, GATHER_ACTION, 70, 15)
    );
    // gone: the gather command takes nothing
    let mut u = idle(CIVILIAN);
    let (r, fx, _) = run_order(&mut u, queued(gather, 99), 4, &mut side);
    assert_eq!((r, fx), (Ok(0), Vec::new()));
    // the hunt stops first, then hunts; on a gone target the stop stays
    let mut u = UnitAi {
        current_action: GATHER_ACTION,
        ..idle(CIVILIAN)
    };
    let (r, fx, _) = run_order(&mut u, queued(HUNT_ORDER, 70), 4, &mut side);
    assert_eq!((r, fx), (Ok(1), vec![Effect::HuntObject(70)]));
    assert_eq!(
        (u.current_order, u.current_action),
        (HUNT_ORDER, HUNT_ACTION)
    );
    let mut u = UnitAi {
        current_action: GATHER_ACTION,
        ..idle(CIVILIAN)
    };
    let (r, fx, _) = run_order(&mut u, queued(HUNT_ORDER, 99), 4, &mut side);
    assert_eq!((r, fx), (Ok(0), vec![Effect::Stop]));
    assert_eq!((u.current_order, u.current_action), (-1, -1));
    // the auto boar luring hunts, then holds 0x2be either way
    let luring = ai_kinds::ORDER_AUTO_BOAR_LURING;
    let mut u = idle(CIVILIAN);
    let (r, _, _) = run_order(&mut u, queued(luring, 70), 4, &mut side);
    assert_eq!(r, Ok(1));
    assert_eq!(
        (u.current_order, u.current_action, u.last_action),
        (luring, BOAR_LURING_ACTION, HUNT_ACTION)
    );
    let mut u = idle(CIVILIAN);
    let (r, _, _) = run_order(&mut u, queued(luring, 99), 4, &mut side);
    assert_eq!(
        (r, u.current_order, u.current_action),
        (Ok(0), -1, BOAR_LURING_ACTION)
    );
}

#[test]
fn the_move_orders_move_to_their_point_within_their_range() {
    let mut side = OrderSide::default();
    for order in [MOVE_ORDER, ai_kinds::ORDER_GROUP_GATHER] {
        let mut u = idle(SOLDIER_AI);
        let (r, fx, _) = run_order(&mut u, queued(order, -1), 6, &mut side);
        assert_eq!(
            (r, fx),
            (
                Ok(1),
                vec![Effect::Move(AiMove {
                    point: [12.7, 30.2, 0.5],
                    range: 3.0,
                    force: true,
                    walk: true,
                })]
            )
        );
        assert_eq!((u.current_order, u.current_action), (order, MOVE_ACTION));
    }
}

#[test]
fn the_follow_order_follows_and_moves_to_a_target_beyond_its_range() {
    // the gap (1) within the range squared (9): no move
    let mut side = OrderSide::holding(&[(70, 6)]);
    let mut u = idle(SOLDIER_AI);
    let (r, fx, _) = run_order(&mut u, queued(FOLLOW_ORDER, 70), 6, &mut side);
    assert_eq!((r, fx), (Ok(1), Vec::new()));
    assert_eq!(
        (
            u.current_order,
            u.current_action,
            u.current_target,
            u.desired_distance
        ),
        (FOLLOW_ORDER, FOLLOW_ACTION, 70, 3.0)
    );
    // beyond it: the move to the object (slot 0x138)
    side.world[0].gap_squared = 9.5;
    let mut u = idle(SOLDIER_AI);
    let (r, fx, _) = run_order(&mut u, queued(FOLLOW_ORDER, 70), 6, &mut side);
    assert_eq!(r, Ok(1));
    assert_eq!(
        fx,
        vec![Effect::MoveToObject {
            target: 70,
            state: 2,
            range: 3.0,
        }]
    );
    assert_eq!(
        (u.current_action, u.last_action),
        (MOVE_ACTION, FOLLOW_ACTION)
    );
    // a gap that is not a number stays (COMISS, JBE); a gone target is not taken
    side.world[0].gap_squared = f32::NAN;
    let mut u = idle(SOLDIER_AI);
    assert_eq!(
        run_order(&mut u, queued(FOLLOW_ORDER, 70), 6, &mut side).1,
        Vec::new()
    );
    let mut u = idle(SOLDIER_AI);
    assert_eq!(
        run_order(&mut u, queued(FOLLOW_ORDER, 99), 6, &mut side).0,
        Ok(0)
    );
}

#[test]
fn the_transport_enter_and_unload_orders_give_the_unit_s_commands() {
    let mut side = OrderSide::holding(&[(70, 3)]);
    let transport = ai_kinds::ORDER_TRANSPORT;
    let mut u = idle(11);
    let (r, _, _) = run_order(&mut u, queued(transport, -1), 20, &mut side);
    assert_eq!(r, Ok(1));
    assert_eq!(side.commands, [UnitCommand::Transport([12.7, 30.2, 0.5])]);
    assert_eq!(
        (
            u.current_order,
            u.current_action,
            u.current_target,
            u.target_point
        ),
        (transport, TRANSPORT_ACTION, -1, [12.7, 30.2, 0.5])
    );
    // the enter: the target at (-1, -1, -1) with its class
    let mut side = OrderSide::holding(&[(70, 3)]);
    let enter = crate::garrison::ENTER_ORDER;
    let mut u = idle(SOLDIER_AI);
    let (r, _, _) = run_order(&mut u, queued(enter, 70), 6, &mut side);
    assert_eq!(r, Ok(1));
    assert_eq!(side.commands, [UnitCommand::Enter(70)]);
    assert_eq!(
        (
            u.current_action,
            u.current_target,
            u.target_type,
            u.target_point
        ),
        (crate::garrison::ENTER_ACTION, 70, 3, [-1.0; 3])
    );
    // a villager takes no enter into an object the world does not hold; others ask
    let mut u = idle(CIVILIAN);
    assert_eq!(run_order(&mut u, queued(enter, 99), 4, &mut side).0, Ok(0));
    assert_eq!(side.commands.len(), 1);
    // (FUN_140c64c10 keeps the class it held for an object the world does not hold)
    let mut u = UnitAi {
        target_type: 12,
        ..idle(SOLDIER_AI)
    };
    assert_eq!(run_order(&mut u, queued(enter, 99), 6, &mut side).0, Ok(1));
    assert_eq!((u.current_target, u.target_type), (99, 12));
    // the unloads: the point at the unit's height; the unit itself the target
    for (order, over_wall, action) in [
        (ai_kinds::ORDER_UNLOAD, false, UNLOAD_ACTION),
        (
            ai_kinds::ORDER_UNLOAD_OVER_WALL,
            true,
            UNLOAD_OVER_WALL_ACTION,
        ),
    ] {
        let mut side = OrderSide::default();
        let mut u = idle(11);
        let (r, _, _) = run_order(&mut u, queued(order, -1), 20, &mut side);
        assert_eq!(r, Ok(1));
        assert_eq!(
            side.commands,
            [UnitCommand::Unload {
                to: [12.7, 30.2, 0.25],
                over_wall,
            }]
        );
        assert_eq!(
            (
                u.current_order,
                u.current_action,
                u.current_target,
                u.target_type
            ),
            (order, action, 9, 20)
        );
        assert_eq!(u.target_point, [5.0, 6.0, 0.25]);
    }
}

#[test]
fn seek_and_destroy_attacks_and_explore_and_destroy_explores_at_tiles() {
    let mut side = OrderSide::holding(&[(70, 6)]);
    let seek = ai_kinds::ORDER_SEEK_AND_DESTROY;
    let mut u = idle(SOLDIER_AI);
    let (r, fx, _) = run_order(&mut u, queued(seek, 70), 6, &mut side);
    assert_eq!((r, fx), (Ok(1), Vec::new()));
    assert_eq!(side.attacks, [(70, true)]);
    assert_eq!(
        (
            u.current_order,
            u.current_action,
            u.current_target,
            u.target_type
        ),
        (seek, SEEK_ACTION, 70, 6)
    );
    assert_eq!(u.target_point, [12.0, 30.0, EXPLORE_Z]);
    let destroy = ai_kinds::ORDER_EXPLORE_AND_DESTROY;
    let mut u = idle(SOLDIER_AI);
    let (r, fx, _) = run_order(&mut u, queued(destroy, -1), 6, &mut side);
    assert_eq!((r, fx), (Ok(1), vec![Effect::Explore([12.0, 30.0])]));
    assert_eq!(
        (
            u.current_order,
            u.current_action,
            u.current_target,
            u.target_point
        ),
        (destroy, EXPLORE_DESTROY_ACTION, -1, [12.0, 30.0, EXPLORE_Z])
    );
}

#[test]
fn the_wait_order_waits_and_leaves_a_patrol_order() {
    // case 0x2d8: the action value 0x274 (the last stays), the patrol order's timers, then
    // the order; an adjusted idle timeout of 750 is redrawn (one draw)
    let mut side = OrderSide::default();
    let mut u = UnitAi {
        current_order: PATROL,
        current_action: MOVE_ACTION,
        last_action: 5,
        adjusted_idle_timeout: 750,
        idle_timeout: 3000,
        secondary_timer: 900,
        ..idle(SOLDIER_AI)
    };
    let wait = ai_kinds::ORDER_WAIT;
    let (r, fx, draws) = run_order(&mut u, queued(wait, -1), 6, &mut side);
    assert_eq!((r, fx, draws), (Ok(1), Vec::new(), 1));
    assert_eq!(
        (
            u.current_order,
            u.order_priority,
            u.current_action,
            u.last_action
        ),
        (wait, 40, WAIT_ACTION, 5)
    );
    assert_eq!(u.secondary_timer, 750);
    assert_ne!(u.adjusted_idle_timeout, 750);
}

#[test]
fn the_attack_roundup_passes_its_own_target_and_one_it_cannot_reach() {
    let mut side = OrderSide::holding(&[(70, 6)]);
    let roundup = ai_kinds::ORDER_ATTACK_ROUNDUP;
    let mut u = UnitAi {
        current_target: 70,
        ..idle(SOLDIER_AI)
    };
    assert_eq!(
        run_order(&mut u, queued(roundup, 70), 6, &mut side).0,
        Ok(0)
    );
    // with an action value it must reach the target
    let mut u = UnitAi {
        current_action: MOVE_ACTION,
        ..idle(SOLDIER_AI)
    };
    assert_eq!(
        run_order(&mut u, queued(roundup, 70), 6, &mut side).0,
        Ok(0)
    );
    assert_eq!(side.attacks, Vec::new());
    // without one it attacks, and the order becomes 700, leaving a patrol order
    let mut u = UnitAi {
        current_order: PATROL,
        secondary_timer: 900,
        ..idle(SOLDIER_AI)
    };
    assert_eq!(
        run_order(&mut u, queued(roundup, 70), 6, &mut side).0,
        Ok(1)
    );
    assert_eq!(
        (u.current_order, u.current_action, u.secondary_timer),
        (ATTACK_ORDER, ATTACK_ACTION, 750)
    );
}

#[test]
fn the_order_on_a_target_goes_to_the_caller_or_moves_to_a_gone_one_s_point() {
    let mut side = OrderSide::holding(&[(70, 6)]);
    let o = queued(ORDER_ON_TARGET, 70);
    let mut u = idle(CIVILIAN);
    let (r, fx, _) = run_order(&mut u, o, 4, &mut side);
    assert_eq!((r, fx), (Ok(1), vec![Effect::OrderOnTarget(o)]));
    assert_eq!(u.current_order, -1);
    // gone, without its type: the move to the point, and the order on a target the AI's
    for order in [ORDER_ON_TARGET, ORDER_ON_TARGET_2] {
        let mut u = idle(CIVILIAN);
        let (r, fx, _) = run_order(&mut u, queued(order, 99), 4, &mut side);
        assert_eq!(r, Ok(1));
        assert_eq!(
            fx,
            vec![Effect::Move(AiMove {
                point: [12.7, 30.2, 0.5],
                range: 3.0,
                force: true,
                walk: true,
            })]
        );
        assert_eq!((u.current_order, u.order_priority), (order, 40));
    }
}

#[test]
fn a_gone_target_of_a_type_sends_the_unit_to_the_closest_resource_of_its_kind() {
    // the order's type a class (no owner): a tree's (15, wood: kind 0, fewer than 2
    // workers), searched from the order's shown point at any state; the unit works what it
    // finds (slot 0x7e8) and the order is not taken
    let tree = Gatherable {
        id: 80,
        position: [13.5, 31.5, 0.0],
        class: 15,
    };
    let mut side = OrderSide {
        resource: Some(tree),
        ..OrderSide::default()
    };
    let typed = QueuedOrder {
        target_type: 15,
        ..queued(ORDER_ON_TARGET, 99)
    };
    let mut u = UnitAi {
        current_target: 7,
        ..idle(CIVILIAN)
    };
    let before = u.clone();
    let (r, fx, _) = run_order(&mut u, typed, 4, &mut side);
    assert_eq!(
        (r, fx),
        (
            Ok(0),
            vec![Effect::WorkOn {
                target: 80,
                point: [13.5, 31.5, 0.0],
            }]
        )
    );
    assert_eq!(u, before);
    assert_eq!(
        side.searches,
        [ResourceSearch {
            kind: 0,
            cap: 2,
            site: -1,
            current: 7,
            point: [12.5, 30.5],
            min_state: 0,
        }]
    );
    // with an owner the type is a master of that player's: a gold mine's class (32: kind
    // 4, any workers); a master the player does not hold, or nothing found, gives nothing
    let mut side = OrderSide {
        resource: Some(tree),
        classes: vec![((2, 66), 32)],
        ..OrderSide::default()
    };
    let owned = |master| QueuedOrder {
        target_type: master,
        target_owner: 2,
        ..queued(ORDER_ON_TARGET, 99)
    };
    let mut u = idle(CIVILIAN);
    assert_eq!(run_order(&mut u, owned(66), 4, &mut side).0, Ok(0));
    assert_eq!((side.searches[0].kind, side.searches[0].cap), (4, -1));
    let (r, fx, _) = run_order(&mut u, owned(67), 4, &mut side);
    assert_eq!((r, fx, side.searches.len()), (Ok(0), Vec::new(), 1));
    side.resource = None;
    let (r, fx, _) = run_order(&mut u, owned(66), 4, &mut side);
    assert_eq!((r, fx, side.searches.len()), (Ok(0), Vec::new(), 2));
    // a class of no kind (a soldier's 6), or a unit AI type whose slot 0x1b0 gives none
    let mut side = OrderSide::default();
    let soldier_type = QueuedOrder {
        target_type: 6,
        ..queued(ORDER_ON_TARGET, 99)
    };
    let mut u = idle(CIVILIAN);
    assert_eq!(run_order(&mut u, soldier_type, 4, &mut side).0, Ok(0));
    let mut u = idle(SOLDIER_AI);
    assert_eq!(run_order(&mut u, typed, 6, &mut side).0, Ok(0));
    assert_eq!(side.searches, Vec::new());
}

#[test]
fn slot_0x1b0_gives_the_kind_of_resource_by_the_unit_ai_s_type() {
    let kind = |ai_type, class| idle(ai_type).resource_kind(class);
    for ai_type in [CIVILIAN, 17] {
        assert_eq!(
            [15, 7, 5, 31, 33, 8, 32, 63, 6].map(|c| kind(ai_type, c)),
            [0, 1, 2, 2, 2, 3, 4, 4, -1]
        );
    }
    assert_eq!(
        [5, 31, 33, 63, 15, 32, 34].map(|c| kind(FISHING_AI, c)),
        [2, 2, 2, 4, -1, -1, -1]
    );
    assert_eq!([15, 5, 63].map(|c| kind(SOLDIER_AI, c)), [-1, -1, -1]);
}

#[test]
fn the_attack_move_attacks_with_a_trebuchet_and_moves_otherwise() {
    let mut side = OrderSide::holding(&[(70, 3)]);
    for class in [TREBUCHET_CLASS, TREBUCHET_PACKED_CLASS] {
        let mut u = idle(SIEGE_AI);
        let (r, fx, _) = run_order(&mut u, queued(ATTACK_MOVE, 70), class, &mut side);
        assert_eq!((r, fx), (Ok(1), Vec::new()));
        assert_eq!(
            (u.current_order, u.current_action),
            (ATTACK_ORDER, ATTACK_ACTION)
        );
    }
    for (class, target) in [(6, 70), (TREBUCHET_CLASS, -1)] {
        let mut u = idle(SOLDIER_AI);
        let (r, fx, _) = run_order(&mut u, queued(ATTACK_MOVE, target), class, &mut side);
        assert_eq!(r, Ok(1));
        assert!(matches!(fx[..], [Effect::Move(_)]));
        assert_eq!(
            (u.current_order, u.current_action),
            (ATTACK_MOVE, MOVE_ACTION)
        );
    }
}

#[test]
fn the_order_tail_takes_the_order_and_leaves_a_patrol_order() {
    let patrolling = |order| UnitAi {
        current_order: order,
        order_priority: 7,
        adjusted_idle_timeout: 750,
        idle_timeout: 3000,
        secondary_timer: 2000,
        current_target: 70,
        ..idle(SOLDIER_AI)
    };
    // leaving a patrol order for another redraws a timeout of exactly 750 and caps the
    // secondary timer
    let mut u = patrolling(PATROL);
    let mut r = rng();
    assert_eq!(u.order_tail(MOVE_ORDER, 40, 6, false, &mut r), 1);
    assert_eq!(
        (
            u.current_order,
            u.order_priority,
            u.secondary_timer,
            r.draws
        ),
        (MOVE_ORDER, 40, 750, 1)
    );
    // staying on it does nothing to the timers
    let mut u = patrolling(ATTACK_MOVE);
    let mut r = rng();
    let _ = u.order_tail(ATTACK_MOVE, 40, 6, false, &mut r);
    assert_eq!(
        (u.secondary_timer, u.adjusted_idle_timeout, r.draws),
        (2000, 750, 0)
    );
    // the stop ends the order and its priority
    let mut u = patrolling(PATROL);
    let _ = u.order_tail(STOP_ORDER, 40, 6, false, &mut rng());
    assert_eq!(
        (u.current_order, u.order_priority, u.secondary_timer),
        (-1, -1, 750)
    );
    // the attack move's attack and the roundup become the attack order
    let mut u = patrolling(ATTACK_MOVE);
    let _ = u.order_tail(ATTACK_MOVE, 40, 51, true, &mut rng());
    assert_eq!((u.current_order, u.secondary_timer), (ATTACK_ORDER, 750));
    // a class-18 unit's attack order: the target goes, the order is -1
    let mut u = patrolling(-1);
    let _ = u.order_tail(ATTACK_ORDER, 40, CLASS_18, false, &mut rng());
    assert_eq!(
        (u.current_order, u.order_priority, u.current_target),
        (-1, 40, -1)
    );
}

#[test]
fn the_attack_command_runs_a_pending_stop_first() {
    // the attack command runs at once: each stop pending runs first (the second on an empty
    // list with its standing sprite shown already); a stop with another command pending is
    // not modelled
    let mut u = idle(BOAR_AI);
    let mut side = OrderSide::holding(&[(70, 6)]);
    let at = Surroundings::default();
    let mut fx = vec![Effect::Stop, Effect::Stop];
    assert_eq!(
        u.attack_command(70, true, &at, &mut side, &mut fx),
        Ok(true)
    );
    assert_eq!((fx, side.stops, u.commanded), (Vec::new(), 2, true));
    let mut fx = vec![Effect::Stop, Effect::GatherObject(1)];
    assert!(u.attack_command(70, true, &at, &mut side, &mut fx).is_err());
}

/// The villager's handler (`FUN_140c22bf0`) on the attack action (600): a moved target
/// (0x1fc) with `+0x1a8` set posts the Attack's failure to itself (the unit's slot 0x328:
/// 0x1f9 with 600, 0, 0), else an invalidation, a moved target or a search goes to the base
/// handler (a moved target without a saved patrol: nothing).
#[test]
fn a_villager_s_attack_target_that_moved_is_the_base_handler_s_or_its_failure() {
    let attacking = |stop_after_kill| UnitAi {
        current_order: ATTACK_ORDER,
        current_action: ATTACK_ACTION,
        current_target: 18842,
        target_type: 47,
        stop_after_kill,
        ..idle(CIVILIAN)
    };
    let at = Surroundings {
        id: 6116,
        state: 2,
        ..Surroundings::default()
    };
    let moved = Notify::own(6116, TARGET_MOVED, [0, 0, 0]);
    let mut u = attacking(true);
    let (r, fx, _) = handled(&mut u, moved, &at, false);
    assert_eq!((r, fx), (Ok(GO_ON), Vec::new()));
    assert_eq!(
        u.notifies,
        [Notify::own(6116, ACTION_FAILED, [ATTACK_ACTION, 0, 0])]
    );
    let mut u = attacking(false);
    let (r, fx, _) = handled(&mut u, moved, &at, false);
    assert_eq!((r, fx), (Ok(GO_ON), Vec::new()));
    assert_eq!(u.notifies, Vec::new());
    assert_eq!(u.current_action, ATTACK_ACTION);
    // the base handler's invalidation of the current action stops it
    let mut u = attacking(false);
    let invalid = Notify::own(6116, ACTION_INVALIDATED, [ATTACK_ACTION, 0, 0]);
    let (r, _, _) = handled(&mut u, invalid, &at, false);
    assert!(r.is_ok());
    assert_eq!(u.current_action, -1);
}

/// The villager's case 0x21c (`FUN_140c22bf0`), a load of another resource asked off: the
/// target of its top action (slot 0x190), then the drop-site table's building
/// (`FUN_141658e20`), which the unit works and whose way back the caller queues; without a
/// building nothing. The processing goes on.
#[test]
fn a_villager_drops_another_resource_at_the_table_s_building() {
    let at = Surroundings {
        id: 20,
        state: 2,
        ..Surroundings::default()
    };
    let other = Notify::own(20, OTHER_LOAD, [0; 3]);
    let mut side = Recorder {
        drop_site: Some(45),
        top: Some(77),
        ..Recorder::default()
    };
    let mut fx = Vec::new();
    let r = idle(CIVILIAN).handle(other, 0, &at, false, &mut rng(), &mut side, &mut fx);
    assert_eq!(r, Ok(GO_ON));
    assert_eq!(
        fx,
        [Effect::DropElsewhere {
            site: 45,
            back: Some(77)
        }]
    );
    let mut none = Recorder::default();
    let mut fx = Vec::new();
    let r = idle(CIVILIAN).handle(other, 0, &at, false, &mut rng(), &mut none, &mut fx);
    assert_eq!((r, fx), (Ok(GO_ON), Vec::new()));
}

/// The villager's case 0x21d (`FUN_140c22bf0`): off the build or the hunt action the object
/// its action made (429381190's villager 15295 and its pasture 15342 in update 1386), which
/// the world holds, the unit works at its place (slot 0x7e8); off another action nothing.
/// The processing goes on.
#[test]
fn a_villager_works_the_pasture_it_built() {
    let pasture = UnitSeen {
        id: 15342,
        position: [136.0, 72.0, 0.0],
        ..UnitSeen::default()
    };
    let at = Surroundings {
        id: 15295,
        state: 2,
        units: vec![pasture],
        ..Surroundings::default()
    };
    for action in [BUILD_ACTION, HUNT_ACTION] {
        let built = Notify::own(15295, PASTURE_BUILT, [action, 15342, 0]);
        let (r, fx, _) = handled(&mut idle(CIVILIAN), built, &at, false);
        assert_eq!(r, Ok(GO_ON));
        assert_eq!(
            fx,
            [Effect::WorkOn {
                target: 15342,
                point: [136.0, 72.0, 0.0]
            }]
        );
    }
    let other = Notify::own(15295, PASTURE_BUILT, [GATHER_ACTION, 15342, 0]);
    assert_eq!(
        handled(&mut idle(CIVILIAN), other, &at, false).1,
        Vec::new()
    );
}

/// The base handler's completion (`FUN_140c589c0` case 0x1fa; the villager's own case takes
/// it before) of an attack on the attack order 700: processIdle with 0; when it produced an action the processing breaks (0xd),
/// else the target goes and slot 0xb0 with 0 ends the order. A unit without an order in a
/// group with a command keeps its place as its state position.
#[test]
fn an_attack_completed_on_the_attack_order_looks_for_the_next_or_ends_it() {
    let done = Notify::own(9, ACTION_COMPLETED, [ATTACK_ACTION, 0, 0]);
    // a transport ship's processIdle only caps its last human order time (5)
    let mut u = UnitAi {
        current_order: ATTACK_ORDER,
        order_priority: 100,
        current_action: ATTACK_ACTION,
        current_target: 70,
        ..idle(11)
    };
    let here = Surroundings {
        position: [5.0, 6.0, 0.0],
        class: 20,
        ..Surroundings::default()
    };
    let (r, fx, _) = handled(&mut u, done, &here, false);
    assert_eq!(r, Ok(GO_ON));
    assert_eq!(fx, [Effect::CapHumanOrder(HUMAN_ORDER_CAP), Effect::Stop]);
    assert_eq!(
        (u.current_order, u.order_priority, u.last_order),
        (-1, -1, ATTACK_ORDER)
    );
    assert_eq!(
        (u.current_target, u.last_target, u.current_action),
        (-1, 70, -1)
    );
    assert_eq!(u.state_position, [5.0, 6.0]);
    // a boar beyond its leash heads home (6): the processing breaks there
    let mut u = UnitAi {
        current_order: ATTACK_ORDER,
        current_action: ATTACK_ACTION,
        current_target: 70,
        ..boar()
    };
    let mut fx = Vec::new();
    let r = u.handle(
        done,
        0,
        &at([43.5, 105.5, 0.0]),
        false,
        &mut seed_for(false),
        &mut Recorder::default(),
        &mut fx,
    );
    assert_eq!(r, Ok(PRODUCED));
    assert!(matches!(
        fx[..],
        [Effect::CapHumanOrder(HUMAN_ORDER_CAP), Effect::Move(_)]
    ));
    // (its move forgot the target: FUN_140c64c10 with -1)
    assert_eq!(
        (u.current_order, u.current_target, u.last_target),
        (ATTACK_ORDER, -1, 70)
    );
    // without an order, in a group with a command: the state position, then the stop
    let mut u = UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 70,
        ..idle(SOLDIER_AI)
    };
    let grouped = Surroundings {
        group: Some(GroupState {
            commanded: true,
            ..GroupState::default()
        }),
        ..here.clone()
    };
    let (r, fx, _) = handled(&mut u, done, &grouped, false);
    assert_eq!((r, fx), (Ok(GO_ON), vec![Effect::Stop]));
    assert_eq!(u.state_position, [5.0, 6.0]);
    let mut u = UnitAi {
        current_action: ATTACK_ACTION,
        current_target: 70,
        ..idle(SOLDIER_AI)
    };
    let _ = handled(&mut u, done, &here, false);
    assert_eq!(u.state_position, [0.0, 0.0]);
}
