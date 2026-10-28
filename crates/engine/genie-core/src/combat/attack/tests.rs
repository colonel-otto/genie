use super::*;
use crate::objects::layers::{Animated, Body};
use crate::unit_ai::OrderKind;

/// `FUN_140a82ae0` case 6 after the hit: a ranged unit whose attack timer ran out is done
/// without a target and ready again past its interrupt frame (its firing frame without one);
/// at or past the interrupt frame readyToAttack runs, the swing going on.
#[test]
fn a_swing_after_its_hit_follows_its_interrupt_and_firing_frames() {
    use AfterHit::{Continue, Done, Ready};
    // ranged, timer run out (NaN counts too)
    assert_eq!(after_hit(true, 0.0, false, 5, 3, 9), Done);
    assert_eq!(after_hit(true, 0.0, true, 5, 3, 6), Ready);
    assert_eq!(after_hit(true, f32::NAN, true, 5, 3, 6), Ready);
    assert_eq!(after_hit(true, -1.0, true, -1, 3, 4), Ready);
    assert_eq!(
        after_hit(true, 0.0, true, -1, 3, 3),
        Continue { check: false }
    );
    assert_eq!(
        after_hit(true, 0.0, true, 5, 3, 5),
        Continue { check: true }
    );
    // ranged with time left, or melee: only the interrupt test
    assert_eq!(
        after_hit(true, 0.5, false, 5, 3, 9),
        Continue { check: true }
    );
    assert_eq!(
        after_hit(false, 0.0, false, 5, 3, 4),
        Continue { check: false }
    );
    assert_eq!(
        after_hit(false, 0.0, true, 5, 3, 5),
        Continue { check: true }
    );
    assert_eq!(
        after_hit(false, 0.0, true, -1, 3, 9),
        Continue { check: false }
    );
}

/// A unit's own Attack on `target` in `state` with range `range`.
pub(crate) fn own_attack(target: i32, state: u8, range: f32) -> Attack {
    Attack {
        target,
        target2: -1,
        state,
        timer: 0.0,
        range,
        min_range: 0.0,
        point: [NONE; 3],
        seen: [NONE; 3],
        goto: [NONE; 3],
        move_to: None,
        move_wait: MoveWait::default(),
        pending: false,
        swing: 0.0,
        rest: 0.0,
        arms: Arms {
            move_sprite: None,
            sprite: None,
            second_sprite: None,
            idle_sprite: None,
            idle_attack_sprite: None,
            missile: -1,
            fire_frame: 0,
            interrupt_frame: -1,
        },
        sub_action: false,
        repaths: false,
        runs: true,
        same_owner: false,
    }
}

/// Slot 0x38 (`FUN_140a82ac0`): `set_state(1)`: its `MoveTo` goes, stopping the unit and its
/// animation, then the animation runs at speed 1 and the unit stands; the target stays.
#[test]
fn a_stop_ends_the_attack_and_the_unit_stands() {
    let stand = Graphic {
        id: 1,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 0,
    };
    let mut a = own_attack(77, MOVE, 1.0);
    a.move_to = Some(MoveTo::Moving);
    a.arms.idle_sprite = Some(stand);
    let mut body = Body::Moving(crate::gathering::hunt::tests::walker());
    body.set_anim_speed(0.5);
    let (mut sprite, mut rng) = (None, Well512::default());
    a.halt(&mut body, &mut sprite, &mut rng).unwrap();
    // the MoveTo's teardown stops the unit (its speed 0 at once), then set_state(1) sets 1
    assert_eq!(
        (a.state, a.move_to, a.target, body.anim_speed()),
        (DONE, None, 77, Some(1.0))
    );
    assert_eq!(sprite.map(|s| s.graphic.id), Some(1));
    assert_eq!(body.mover().unwrap().velocity.unwrap().velocity[2], 0.0);
    // a unit without a standing sprite is not modelled
    let mut bare = own_attack(77, READY, 1.0);
    assert!(bare.halt(&mut Body::Static, &mut sprite, &mut rng).is_err());
}

/// An object of class 9 in state 2 at `x` on the line y = 10, radii 0.2.
fn mark(id: i32, x: f32) -> Mark {
    Mark {
        id,
        kind: 70,
        class: 9,
        state: 2,
        position: [x, 10.0, 0.5],
        radius: [0.2, 0.2],
    }
}

/// A lion (speed 1.2, radii 0.4) at (10, 10) without an area effect.
const LION: Wielder = Wielder {
    area_effect: 0.0,
    speed: 1.2,
    position: [10.0, 10.0, 0.0],
    radius: [0.4, 0.4],
};

/// `FUN_140a841d0`: the Attack's own target in states 4 to 7 or 11 is kept as it is; in
/// another state a unit that may move takes it again: where it was seen becomes its place,
/// and the same target changes nothing else.
#[test]
fn an_attack_keeps_its_own_target_and_sees_it_again_out_of_its_moving_states() {
    let deer = mark(7, 13.0);
    for state in [4, 5, 6, 7, 11] {
        let mut a = own_attack(7, state, 0.0);
        let w = a.work(Some(deer), None, &LION, |_| panic!("no range test"));
        assert_eq!(
            w,
            Worked {
                kept: true,
                ..Worked::default()
            }
        );
        assert_eq!(a.seen, [NONE; 3], "{state}");
    }
    for state in [1, 2, 3, 13] {
        let mut a = own_attack(7, state, 0.0);
        let w = a.work(Some(deer), None, &LION, |id| id == 7);
        assert_eq!(
            w,
            Worked {
                kept: true,
                ..Worked::default()
            }
        );
        assert_eq!((a.target, a.seen), (7, deer.position), "{state}");
    }
    // none answers false
    let mut a = own_attack(7, 1, 0.0);
    assert_eq!(a.work(None, None, &LION, |_| true), Worked::default());
}

/// The range test of slot 9: a unit without speed, or whose unit AI may not move to attack,
/// keeps a target only within its reach (the range, half a tile more up to 0.1; the gap
/// between the footprints); a NaN speed asks the unit AI, a NaN gap is within (`JA`).
#[test]
fn an_attack_that_cannot_follow_keeps_a_target_only_in_reach() {
    // the gap: 13 - 10 - (0.4 + 0.2) = 2.4
    let deer = mark(7, 13.0);
    let reach = |range: f32, speed: f32, can: bool| {
        let mut a = own_attack(7, 1, range);
        let unit = Wielder { speed, ..LION };
        a.work(Some(deer), None, &unit, |_| can).kept
    };
    assert!(reach(0.0, 1.2, true));
    assert!(!reach(2.0, 1.2, false));
    assert!(!reach(2.0, 0.0, true));
    assert!(reach(2.4, 0.0, true));
    assert!(reach(2.5, 0.0, false));
    // a range up to 0.1 reaches half a tile more: 0.1 + 0.5 against a gap of 0.6
    let near = mark(7, 11.2);
    let mut a = own_attack(7, 1, 0.1);
    assert!(
        a.work(Some(near), None, &Wielder { speed: 0.0, ..LION }, |_| true)
            .kept
    );
    let mut a = own_attack(7, 1, 0.0);
    let farther = mark(7, 11.3);
    assert!(
        !a.work(Some(farther), None, &Wielder { speed: 0.0, ..LION }, |_| {
            true
        })
        .kept
    );
    // UCOMISS/JP: a NaN speed is not 0, so the unit AI decides
    let mut asked = false;
    let mut a = own_attack(7, 1, 0.0);
    let unit = Wielder {
        speed: f32::NAN,
        ..LION
    };
    assert!(
        a.work(Some(deer), None, &unit, |_| {
            asked = true;
            true
        })
        .kept
    );
    assert!(asked);
    // COMISS/JA: a NaN gap does not lie beyond the reach
    let lost = Mark {
        position: [f32::NAN, 10.0, 0.0],
        ..deer
    };
    let mut a = own_attack(7, 1, 0.0);
    assert!(
        a.work(Some(lost), None, &Wielder { speed: 0.0, ..LION }, |_| false)
            .kept
    );
}

/// An area effect's Attack takes a building's main building (`+0xbb8`): another target than
/// its own clears the unit's attack count, lets the old one go and takes the new one (an own
/// Attack; a sub-action does neither); a target in state 7 or more is not taken, and one of
/// class 60 changes nothing but where it was seen.
#[test]
fn an_area_effect_s_attack_takes_a_building_s_main_building() {
    let annex = Mark {
        kind: BUILDING,
        class: 3,
        ..mark(30, 11.0)
    };
    let main = Mark {
        id: 31,
        position: [12.0, 10.0, 0.0],
        radius: [1.5, 1.5],
        ..annex
    };
    let mangonel = Wielder {
        area_effect: 1.0,
        ..LION
    };
    let mut a = own_attack(30, 6, 0.0);
    let w = a.work(Some(annex), Some(main), &mangonel, |id| id == 31);
    assert_eq!(
        w,
        Worked {
            kept: true,
            clears_count: true,
            lets_go: Some(30),
            takes: Some(31),
        }
    );
    assert_eq!((a.target, a.seen), (31, main.position));
    // without an area effect the annex itself is the target, its own in state 6
    let mut a = own_attack(30, 6, 0.0);
    assert_eq!(
        a.work(Some(annex), Some(main), &LION, |_| true),
        Worked {
            kept: true,
            ..Worked::default()
        }
    );
    // a sub-action lets go and takes nothing
    let mut a = own_attack(30, 6, 0.0);
    a.sub_action = true;
    let w = a.work(Some(annex), Some(main), &mangonel, |_| true);
    assert_eq!(
        (w.lets_go, w.takes, w.clears_count, a.target),
        (None, None, true, 31)
    );
    // a new target in state 7 or more leaves the Attack without one
    let mut a = own_attack(30, 6, 0.0);
    let dying = Mark { state: 7, ..main };
    let w = a.work(Some(annex), Some(dying), &mangonel, |_| true);
    assert_eq!((w.lets_go, w.takes, a.target), (Some(30), None, -1));
    // class 60: slot 0x68 returns at once; the seen point still follows
    let mut a = own_attack(7, 1, 0.0);
    let odd = Mark {
        class: CLASS_60,
        ..mark(8, 10.5)
    };
    let w = a.work(Some(odd), None, &LION, |_| true);
    assert_eq!(
        (w, a.target, a.seen),
        (
            Worked {
                kept: true,
                ..Worked::default()
            },
            7,
            odd.position
        )
    );
    // slot 0x68 with none: the count clears and the old target goes
    let mut a = own_attack(7, 1, 0.0);
    assert_eq!(
        a.set_target(None),
        Worked {
            clears_count: true,
            lets_go: Some(7),
            ..Worked::default()
        }
    );
    assert_eq!(a.target, -1);
}

/// `FUN_1413b9f00`: the sprite of the graphic asked for goes back to its first frame (true);
/// another graphic's, or none, is left (false).
#[test]
fn a_sprite_resets_only_for_its_own_graphic() {
    let swing = Graphic {
        id: 224,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let other = Graphic { id: 225, ..swing };
    let mut sprite = Some(ActiveSprite {
        frame: 6,
        last_frame: 5,
        looped: true,
        ..ActiveSprite::start(swing, &mut Well512::default())
    });
    let before = sprite;
    assert!(!reset_sprite(&mut sprite, other));
    assert_eq!(sprite, before);
    assert!(reset_sprite(&mut sprite, swing));
    let s = sprite.unwrap();
    assert_eq!((s.frame, s.last_frame, s.looped), (0, 0, false));
    assert!(!reset_sprite(&mut None, swing));
}

/// A soldier (class 6, type 70, speed 0.8, radius 0.2) as a mover.
fn soldier_mover(speed: f32) -> Mover {
    use crate::movement::{MoveStats, Velocity};
    Mover {
        forward: [1.0, 0.0, 0.0],
        right: [0.0, 0.0, 1.0],
        velocity: Some(Velocity {
            velocity: [0.0, 0.0, 0.0],
            acceleration: [0.0, 0.0, 0.0],
        }),
        paths: vec![],
        factor: 1.0,
        stats: MoveStats {
            fly_mode: 0,
            speed,
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
            walk_speed: speed,
            stand_speed: speed,
            run_speed: speed,
            facets: Some(crate::objects::facing::SpriteFacets {
                count: 8,
                flag_1: true,
            }),
            weapon_range: 0.0,
        },
        waiting: 0,
        wait_delays: 0,
        attack_moving: false,
        last_move: 0,
        moved_at: [0.0; 3],
        substitute_count: 0,
        ignore_units: Vec::new(),
        user_waypoints: Vec::new(),
        last_turn: 0.0,
        angle: 0.0,
        move_to_point: None,
        animated: Animated::default(),
        pauses: Vec::new(),
        in_the_way: Vec::new(),
        sprite_facets: None,
    }
}

const STANDING: Graphic = Graphic {
    id: 300,
    frames: 10,
    facets: 8,
    speed_multiplier: 1.0,
    frame_duration: 0.1,
    replay_delay: 0.0,
    flags: 1,
};

/// What an Attack's update reads besides its walk: the unit's sprite, timers and target.
pub(crate) struct Rig {
    pub(crate) sprite: Option<ActiveSprite>,
    timer: f32,
    reload_worldtime: u32,
    victim: Victim,
    blasted: Vec<Victim>,
    rng: Well512,
}

impl Rig {
    pub(crate) fn new(victim: Victim) -> Self {
        Self {
            sprite: None,
            timer: 0.0,
            reload_worldtime: 0,
            victim,
            blasted: Vec::new(),
            rng: Well512::default(),
        }
    }

    pub(crate) fn strike(&mut self) -> Strike<'_> {
        Strike {
            sprite: &mut self.sprite,
            standing: Some(STANDING),
            volleys: false,
            attack_timer: &mut self.timer,
            reload_worldtime: &mut self.reload_worldtime,
            reload: 2.0,
            hit_chance: 100,
            victim: &mut self.victim,
            blasted: &mut self.blasted,
            sprite_rng: &mut self.rng,
            own: Victim {
                id: 1,
                kind: 70,
                class: 6,
                player: 1,
                ..Victim::default()
            },
            executed: None,
            fire: None,
            can_move: true,
            stance: 0,
            state_position: [0.0; 2],
            posted: Vec::new(),
            queue_empty: true,
            world_time: 0,
            ai_order: Some(None),
            ai_type: None,
            computer: false,
            armed: true,
            volley_under_way: false,
            let_go: None,
            forgets: false,
            sends_home: false,
            swing_sprites: SwingSprites::default(),
        }
    }
}

/// A building of player 2 at `at` (radius 0.5, 100 hit points, no armor, reached by blasts
/// of level 2) with its `building` part.
fn struck_building(id: i32, at: [f32; 3], building: damage::Building) -> Victim {
    Victim {
        id,
        hp: 100.0,
        kind: BUILDING,
        class: 3,
        player: 2,
        position: at,
        radius: [0.5, 0.5],
        blast_defense: 2,
        master_hp: 100,
        armored: Some(damage::Armored {
            armors: vec![(4, 0)],
            terrain_bonus: -1,
            friendly_fire: 1.0,
            ..damage::Armored::default()
        }),
        building: Some(building),
        ..Victim::default()
    }
}

/// `FUN_141084230`'s hit on an annex is its main building's (`FUN_1411cea90`'s forward):
/// the main building among the objects the hit may reach loses it, and the annex takes its
/// hit points.
#[test]
fn a_strike_on_an_annex_hits_its_main_building() {
    let mut annex = struck_building(
        21,
        [10.5, 10.0, 0.0],
        damage::Building {
            main: Some(20),
            ..damage::Building::default()
        },
    );
    let mut blasted = vec![struck_building(
        20,
        [11.5, 10.0, 0.0],
        damage::Building {
            annexes: vec![21],
            ..damage::Building::default()
        },
    )];
    let mut own = Victim {
        id: 1,
        kind: 70,
        class: 6,
        player: 1,
        source: damage::Attacker {
            id: 1,
            kind: 70,
            class: 6,
            missile: -1,
            player: 1,
            ..damage::Attacker::default()
        },
        weapons: vec![(4, 4)],
        ..Victim::default()
    };
    let hit = Hit {
        at: own.position,
        from_z: Some(0.0),
        missile: false,
        chance: 100,
    };
    let struck = strike(
        &mut own,
        &mut annex,
        &mut blasted,
        hit,
        &mut Well512::default(),
    )
    .unwrap();
    assert_eq!(struck.result, 1);
    assert_eq!((blasted[0].hp, annex.hp), (96.0, 96.0));
    assert!(blasted[0].building.as_ref().is_some_and(|b| b.struck));
}

/// A building shooter's missile (429184426's watch tower 25663): its elevation factor is the
/// building's (`FUN_1411cc890`), the combat object's times 1.2 while it holds a specific relic
/// (`+0xc24`), so its hit on a lower target does more than a tower's without one.
#[test]
fn a_building_s_relic_raises_its_missile_s_hit() {
    let hit_by = |relics: [i32; 2]| {
        let mut target = struck_building(21, [10.5, 10.0, 0.0], damage::Building::default());
        let mut own = Victim {
            id: 1,
            kind: BUILDING,
            class: 52,
            player: 1,
            position: [8.0, 10.0, 2.0],
            source: damage::Attacker {
                id: 1,
                kind: BUILDING,
                class: 52,
                missile: 50,
                player: 1,
                ..damage::Attacker::default()
            },
            weapons: vec![(4, 40)],
            building: Some(damage::Building {
                relics,
                ..damage::Building::default()
            }),
            ..Victim::default()
        };
        let hit = Hit {
            at: target.position,
            from_z: Some(2.0),
            missile: true,
            chance: 100,
        };
        let _ = strike(
            &mut own,
            &mut target,
            &mut Vec::new(),
            hit,
            &mut Well512::default(),
        )
        .unwrap();
        target.hp
    };
    assert!(hit_by([0, 1]) < hit_by([0, 0]));
    assert!(hit_by([1, 0]) < hit_by([0, 0]));
    assert!(hit_by([0, 1]) < hit_by([1, 0]));
}

/// `FUN_1412115e0`'s trample: its blast about the unit passes its target but reaches the
/// target's main building (5 of 10 at half the damage), whose blast so far (`+0xb90`,
/// `0x141211933`) the hit on the annex then leaves out: 10 less 5.
#[test]
fn a_trample_s_hit_leaves_out_its_building_target_s_blast_so_far() {
    let mut rig = Rig::new(struck_building(
        21,
        [10.2, 10.0, 0.0],
        damage::Building {
            main: Some(20),
            ..damage::Building::default()
        },
    ));
    rig.blasted = vec![struck_building(
        20,
        [10.5, 10.0, 0.0],
        damage::Building {
            annexes: vec![21],
            ..damage::Building::default()
        },
    )];
    let mut s = rig.strike();
    s.own.position = [10.0, 10.0, 0.0];
    s.own.weapons = vec![(4, 10)];
    s.own.source = damage::Attacker {
        id: 1,
        kind: 70,
        class: 6,
        missile: -1,
        player: 1,
        area_effect: 1.0,
        blast_level: 2,
        area_damage: 0.5,
        ..damage::Attacker::default()
    };
    let a = own_attack(21, READY, 0.0);
    a.hit(&mut s, &mut Well512::default()).unwrap();
    assert_eq!(s.executed, Some(21));
    drop(s);
    assert_eq!((rig.blasted[0].hp, rig.victim.hp), (90.0, 90.0));
    assert_eq!(
        rig.blasted[0].building.as_ref().map(|b| b.blast_taken),
        Some(5.0)
    );
}

/// The blast so far a trample's hit leaves out: a building's own, an annex's main building's
/// (one the hit does not hold is not modelled), none for another object.
#[test]
fn a_trample_reads_a_building_s_blast_so_far_or_its_main_building_s() {
    let building = |main, blast_taken| {
        struck_building(
            21,
            [0.0; 3],
            damage::Building {
                main,
                blast_taken,
                ..damage::Building::default()
            },
        )
    };
    assert_eq!(blast_so_far(&building(None, 3.0), &[]), Ok(3.0));
    let main = building(None, 5.0);
    let main = Victim { id: 20, ..main };
    assert_eq!(blast_so_far(&building(Some(20), 3.0), &[main]), Ok(5.0));
    assert_eq!(
        blast_so_far(&building(Some(20), 3.0), &[]),
        Err(crate::construction::build::NOT_HELD)
    );
    let unit = Victim {
        kind: 70,
        ..Victim::default()
    };
    assert_eq!(blast_so_far(&unit, &[]), Ok(0.0));
}

/// A world holding the target `id` of `player` and class `class` at `at`, radius 0.2.
fn world_with(id: i32, player: usize, class: i16, at: [f32; 3]) -> crate::map::spatial::Spatial {
    let mut world = crate::map::spatial::Spatial::default();
    world.insert(
        id,
        crate::map::spatial::Placed {
            position: at,
            radius: [0.2, 0.2],
            class,
            player,
            master: 4,
            obstruction_type: 0,
            buildable_on: 0,
            tall: false,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    world
}

fn walk_in<'a>(mover: &'a mut Mover, world: &'a mut crate::map::spatial::Spatial) -> Walk<'a> {
    Walk {
        mover: Some(mover),
        me: crate::movement::Me {
            id: 1,
            position: [10.0, 10.0, 0.0],
            action_state: READY,
            group: -1,
        },
        dt: 0.02,
        world,
        plan: None,
    }
}

/// An own Attack of the soldier at (10, 10) on `target` in state 6, range `range` and least
/// range `min`, its point `point`.
fn soldier_attack(target: i32, range: f32, min: f32, point: [f32; 3]) -> Attack {
    let mut a = own_attack(target, READY, range);
    a.min_range = min;
    a.point = point;
    a.arms.idle_sprite = Some(STANDING);
    a
}

/// readyToAttack below the least range: a unit's own Attack posts 0x1fe (−1, −1, −1); a unit
/// that may move waits in its state, one without speed fails (`set_state` 13: the standing
/// sprite at speed 1).
#[test]
fn ready_to_attack_under_the_least_range_posts_0x1fe_and_waits_or_fails() {
    let at = [11.0, 10.0, 0.0];
    let target = || Victim {
        id: 7,
        position: at,
        ..Victim::default()
    };
    for (speed, state) in [(0.8, READY), (0.0, FAILED)] {
        let mut world = world_with(7, 0, 6, at);
        let mut mover = soldier_mover(speed);
        let mut walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(target());
        let mut s = rig.strike();
        let mut a = soldier_attack(7, 4.0, 2.0, [NONE; 3]);
        let r = a.ready_to_attack(&mut walk, &mut s, false).unwrap();
        assert!(matches!(r, Readiness::Waiting));
        assert_eq!((a.state, s.posted.clone()), (state, vec![(0x1fe, [-1; 3])]));
        if state == FAILED {
            assert_eq!(walk.mover.as_deref().map(|m| m.animated.speed), Some(1.0));
            assert_eq!(s.sprite.map(|sp| sp.graphic.id), Some(STANDING.id));
        }
    }
    // a sub-action posts nothing
    let mut world = world_with(7, 0, 6, at);
    let mut mover = soldier_mover(0.8);
    let mut walk = walk_in(&mut mover, &mut world);
    let mut rig = Rig::new(target());
    let mut s = rig.strike();
    let mut a = soldier_attack(7, 4.0, 2.0, [NONE; 3]);
    a.sub_action = true;
    let _ = a.ready_to_attack(&mut walk, &mut s, false).unwrap();
    assert_eq!(s.posted, Vec::new());
}

/// readyToAttack beyond the reach: a unit's own Attack posts 0x1fd; a unit that may move
/// follows (`set_state` 4 is the caller's), one that cannot fails (`set_state` 13).
#[test]
fn ready_to_attack_out_of_reach_follows_or_fails() {
    let at = [15.0, 10.0, 0.0];
    for (speed, can_move, follows) in [(0.8, true, true), (0.0, true, false), (0.8, false, false)] {
        let mut world = world_with(7, 0, 6, at);
        let mut mover = soldier_mover(speed);
        let mut walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            position: at,
            ..Victim::default()
        });
        let mut s = rig.strike();
        s.can_move = can_move;
        let mut a = soldier_attack(7, 1.0, 0.0, [NONE; 3]);
        let r = a.ready_to_attack(&mut walk, &mut s, false).unwrap();
        assert_eq!(s.posted, vec![(0x1fd, [0; 3])]);
        assert_eq!(matches!(r, Readiness::Away(_)), follows);
        assert_eq!(a.state, if follows { READY } else { FAILED });
    }
}

/// readyToAttack on a player's target out of sight: a unit of stance 0 with speed that may
/// move waits (`set_state` 5: a `MoveTo` to where it saw the target); any other fails
/// (`set_state` 13).
/// The update's answer to a target out of sight ([`Attack::out_of_sight`]): the target goes,
/// the attack fails (state 13) and the unit AI forgets it; a predator's boar is sent home, a
/// lion's or one without a unit AI not; a unit's own Attack whose queue is empty posts 0x1f9
/// with 600, a sub-action and a queue with orders not. Another predator's unit AI is not
/// modelled.
#[test]
fn an_attack_on_a_target_out_of_sight_fails() {
    use crate::movement::tests::WalkRig;
    let run = |a: &mut Attack, class, ai_type, queue_empty| {
        let mut walker = WalkRig::on_the_path();
        let mut rig = Rig::new(Victim::default());
        let mut s = rig.strike();
        (s.own.class, s.ai_type, s.queue_empty) = (class, ai_type, queue_empty);
        let r = a.out_of_sight(&mut walker.walk(4), &mut s);
        (r, s.let_go, s.forgets, s.sends_home, s.posted.clone())
    };
    let failed = vec![(crate::unit_ai::ACTION_FAILED, [600, 0, 0])];
    let own_attack = |target, state, range| {
        let mut a = own_attack(target, state, range);
        a.arms.idle_sprite = Some(STANDING);
        a
    };
    let mut a = own_attack(77, MOVE, 1.0);
    let got = run(&mut a, 6, Some(13), true);
    assert_eq!(got, (Ok(3), Some(77), true, false, failed));
    assert_eq!((a.target, a.state), (-1, FAILED));
    let mut a = own_attack(77, MOVE, 1.0);
    assert!(run(&mut a, PREDATOR, Some(BOAR_AI), false).3);
    let mut a = own_attack(77, MOVE, 1.0);
    let lion = run(&mut a, PREDATOR, Some(LION_AI), false);
    assert_eq!((lion.2, lion.3, lion.4), (true, false, vec![]));
    let mut a = own_attack(77, MOVE, 1.0);
    let alone = run(&mut a, PREDATOR, None, true);
    assert_eq!((alone.0, alone.3), (Ok(3), false));
    let mut a = Attack {
        sub_action: true,
        ..own_attack(77, MOVE, 1.0)
    };
    assert_eq!(run(&mut a, 6, Some(13), true).4, vec![]);
    let mut a = own_attack(77, MOVE, 1.0);
    assert!(run(&mut a, PREDATOR, Some(13), true).0.is_err());
}

#[test]
fn ready_to_attack_on_a_target_out_of_sight_fails_or_waits() {
    let at = [11.0, 10.0, 0.0];
    for (stance, fails) in [(0, false), (2, true)] {
        let mut world = world_with(7, 2, 6, at);
        world.sight = Some(crate::vision::visibility::Visibility::new(
            16,
            16,
            Vec::new(),
            vec![0; 256],
            Vec::new(),
            0,
            Vec::new(),
        ));
        let mut mover = soldier_mover(0.8);
        let mut walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            player: 2,
            position: at,
            ..Victim::default()
        });
        let mut s = rig.strike();
        s.stance = stance;
        let mut a = soldier_attack(7, 4.0, 0.0, at);
        a.seen = at;
        let r = a.ready_to_attack(&mut walk, &mut s, false);
        assert!(matches!(r, Ok(Readiness::Waiting)));
        if fails {
            assert_eq!(a.state, FAILED);
        } else {
            assert_eq!(
                (a.state, a.goto, a.move_to),
                (GOTO_SEEN, at, Some(MoveTo::Start))
            );
        }
    }
}

/// The players' sight over a 16×16 map: every tile revealed and explored (seen), or none.
fn sight(seen: bool) -> crate::vision::visibility::Visibility {
    let word = if seen {
        crate::vision::NO_TILE | crate::vision::NO_TILE_EXPLORED
    } else {
        0
    };
    crate::vision::visibility::Visibility::new(
        16,
        16,
        Vec::new(),
        vec![word; 256],
        Vec::new(),
        0,
        Vec::new(),
    )
}

/// `0x140a82ae0`'s moved target: out of its player's sight, a player's unit whose unit AI
/// has an order and an empty queue waits from state 4 (`set_state` 5: a `MoveTo` to where it
/// saw the target, the range slot 0x238's) and returns 0; seen again, an Attack in state 5 or
/// 11 follows the target (`set_state` 4).
#[test]
fn an_attack_waits_where_it_last_saw_its_target_and_follows_it_seen_again() {
    let (seen_at, now_at) = ([12.0, 10.0, 0.0], [13.0, 10.0, 0.0]);
    let run = |state: u8, seen: bool| {
        let mut world = world_with(7, 2, 6, now_at);
        world.sight = Some(sight(seen));
        let mut mover = soldier_mover(0.8);
        let walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            state: 2,
            player: 2,
            position: now_at,
            ..Victim::default()
        });
        let mut s = rig.strike();
        s.ai_order = Some(Some(OrderKind::Attack));
        s.own.source.weapon_range = 3.0;
        let mut a = soldier_attack(7, 1.0, 0.0, seen_at);
        (a.state, a.seen, a.move_to) = (state, seen_at, Some(MoveTo::Moving));
        let code = a.update(walk, &mut s).unwrap();
        (code, a)
    };
    let (code, a) = run(MOVE, false);
    assert_eq!(
        (code, a.state, a.goto, a.seen, a.move_to, a.range),
        (0, GOTO_SEEN, now_at, now_at, Some(MoveTo::Start), 3.0)
    );
    for state in [GOTO_SEEN, GOTO_POINT] {
        let (code, a) = run(state, true);
        assert_eq!(
            (code, a.state, a.move_to, a.range),
            (0, MOVE, Some(MoveTo::Start), 3.0),
            "{state}"
        );
    }
}

/// States 5 and 11 once their `MoveTo` arrived: a target in sight makes the unit ready; out
/// of sight, a human player's unit in state 5 with a range of at least 1 walks onto the
/// point (`set_state` 11), any other lets the target go and fails (`set_state` 13, its own
/// Attack posting 0x1f9) with 3.
#[test]
fn an_attack_arrived_where_it_saw_its_target_looks_again() {
    let at = [13.0, 10.0, 0.0];
    let run = |state: u8, seen: bool, range: f32, computer: bool| {
        let mut world = world_with(7, 2, 6, at);
        world.sight = Some(sight(seen));
        let mut mover = soldier_mover(0.8);
        let walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            state: 2,
            player: 2,
            position: at,
            ..Victim::default()
        });
        let mut s = rig.strike();
        s.computer = computer;
        s.own.source.weapon_range = 4.0;
        let mut a = soldier_attack(7, range, 0.0, at);
        (a.state, a.seen, a.goto, a.move_to) = (state, at, at, Some(MoveTo::Arrived));
        let code = a.update(walk, &mut s).unwrap();
        let (posted, let_go) = (s.posted.clone(), s.let_go);
        (code, a, posted, let_go)
    };
    let (code, a, _, _) = run(GOTO_SEEN, true, 2.0, false);
    assert_eq!(code, 0);
    assert!(!matches!(a.state, GOTO_SEEN | FAILED), "{}", a.state);
    let (code, a, posted, _) = run(GOTO_SEEN, false, 1.0, false);
    assert_eq!(
        (code, a.state, a.goto, a.move_to, a.range, posted),
        (0, GOTO_POINT, at, Some(MoveTo::Start), 4.0, vec![])
    );
    for (state, range, computer) in [
        (GOTO_SEEN, 0.9, false),
        (GOTO_SEEN, f32::NAN, false),
        (GOTO_SEEN, 2.0, true),
        (GOTO_POINT, 2.0, false),
    ] {
        let (code, a, posted, let_go) = run(state, false, range, computer);
        assert_eq!(
            (code, a.state, a.target, let_go, posted),
            (3, FAILED, -1, Some(7), vec![(0x1f9, [600, 0, 0])]),
            "{state} {range} {computer}"
        );
    }
}

/// readyToAttack's ready-at-once test: a target on the attack's point is ready at once, but
/// not with the third argument; a NaN difference from the point is not within half a tile
/// (the reach decides and the target's place becomes the point); a NaN attack timer has run
/// out (slot 0x100's `SETBE`).
#[test]
fn ready_to_attack_s_at_once_test_and_its_nan_paths() {
    let at = [11.0, 10.0, 0.0];
    let run = |point: [f32; 3], skip: bool, range: f32, timer: f32| {
        let mut world = world_with(7, 0, 6, at);
        let mut mover = soldier_mover(0.8);
        let mut walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            position: at,
            ..Victim::default()
        });
        rig.timer = timer;
        let mut s = rig.strike();
        let mut a = soldier_attack(7, range, 0.0, point);
        let r = a.ready_to_attack(&mut walk, &mut s, skip).unwrap();
        (r, a.point)
    };
    assert!(matches!(run(at, false, 0.0, 0.0).0, Readiness::Ready));
    // the third argument skips it: the gap 0.6 lies beyond a reach of 0.5
    assert!(matches!(run(at, true, 0.0, 0.0).0, Readiness::Away(_)));
    // a NaN point: moved and far, the reach decides (2 > 0.6) and the point follows
    let (r, point) = run([f32::NAN, 10.0, 0.0], false, 2.0, 0.0);
    assert!(matches!(r, Readiness::Ready));
    assert_eq!(point, at);
    assert!(matches!(run(at, false, 0.0, f32::NAN).0, Readiness::Ready));
    assert!(matches!(run(at, false, 0.0, 0.5).0, Readiness::Waiting));
}

/// The ready-at-once range test of a unit that does not move: `COMISS`/`JA` past the range,
/// `JBE` on each least-range test, so NaN passes all three.
#[test]
fn the_ready_at_once_range_test_passes_nan() {
    assert!(in_range(1.0, 2.0, 0.0));
    assert!(!in_range(3.0, 2.0, 0.0));
    assert!(in_range(f32::NAN, 2.0, 0.0));
    assert!(in_range(1.0, f32::NAN, 0.0));
    // a least range above 0.1 needs the gap at least as far
    assert!(!in_range(0.5, 2.0, 1.0));
    assert!(in_range(1.5, 2.0, 1.0));
    assert!(in_range(0.5, 2.0, f32::NAN));
    assert!(in_range(f32::NAN, 2.0, 1.0));
}

/// `FUN_140a82ae0`'s states without a case (0, 3, 8 to 10, 12, 14) return 0; their timer
/// runs.
#[test]
fn an_attack_in_a_state_without_a_case_returns_0() {
    let at = [11.0, 10.0, 0.0];
    for state in [0, 3, 8, 9, 10, 12, 14] {
        let mut world = world_with(7, 0, 6, at);
        let mut mover = soldier_mover(0.8);
        let walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            state: 2,
            position: at,
            ..Victim::default()
        });
        let mut s = rig.strike();
        let mut a = soldier_attack(7, 1.0, 0.0, at);
        (a.state, a.seen) = (state, at);
        assert_eq!(a.update(walk, &mut s), Ok(0), "{state}");
        assert_eq!((a.state, a.timer), (state, 0.02));
    }
}

/// `0x140a82ae0` at `LAB_140a834ab`, after the target's sight check: a live target (state 0
/// to 2) of a player's unit (`+0x718` not 0) that its player counts as itself or an ally
/// (`FUN_141663dc0` with 1) ends the Attack (`set_state(1)`; its own Attack posts 0x1fb with
/// 600 and the target) and returns 1, unless the target is livestock (58) or a farm (49) and
/// was its player's when the Attack took it (`+0x9f`). 427003795's update 357: the hunters'
/// goat (class 58), Gaia's when their Attacks began, is now their player's.
#[test]
fn an_attack_on_a_target_its_player_counts_as_its_own_ends() {
    let at = [11.0, 10.0, 0.0];
    let run = |class: i16, allied: bool, same_owner: bool, player: usize, sub_action: bool| {
        let mut world = world_with(7, 1, class, at);
        let mut mover = soldier_mover(0.8);
        let walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            state: 2,
            class,
            player: 1,
            position: at,
            allied: [allied, allied],
            ..Victim::default()
        });
        let mut s = rig.strike();
        s.own.player = player;
        let mut a = soldier_attack(7, 1.0, 0.0, at);
        // state 8 has no case: an Attack the test spares returns 0
        (a.state, a.seen, a.same_owner, a.sub_action) = (8, at, same_owner, sub_action);
        let code = a.update(walk, &mut s).unwrap();
        (code, a.state, s.posted.clone())
    };
    // the hunt's goat, Gaia's when its Attack began: the quiet sub-action ends
    assert_eq!(run(58, true, false, 1, true), (1, DONE, vec![]));
    // a unit's own Attack posts 0x1fb with 600 and its target
    assert_eq!(
        run(58, true, false, 1, false),
        (1, DONE, vec![(0x1fb, [600, 7, 0])])
    );
    // its own livestock or farm when it began: spared
    assert_eq!(run(58, true, true, 1, true).0, 0);
    assert_eq!(run(49, true, true, 1, true).0, 0);
    // another class whatever it began on; an enemy's or Gaia's unit's go on
    assert_eq!(run(6, true, true, 1, true).0, 1);
    assert_eq!(run(6, false, false, 1, true).0, 0);
    assert_eq!(run(6, true, false, 0, true).0, 0);
}

/// Case 1 (state 2) once the swing's sprite is done: the Hunt's Attack, a sub-action, lets
/// its target go (slot 0x68 with none), ends (`set_state` 1) and then posts 0x1fb with 600 and
/// 0x202 before it returns 5.
#[test]
fn a_sub_action_s_attack_after_its_kill_posts_0x1fb_and_returns_5() {
    let at = [11.0, 10.0, 0.0];
    let mut world = world_with(7, 0, 9, at);
    let mut mover = soldier_mover(0.8);
    let walk = walk_in(&mut mover, &mut world);
    let mut rig = Rig::new(Victim {
        id: 7,
        state: 3,
        position: at,
        ..Victim::default()
    });
    let mut s = rig.strike();
    let mut a = soldier_attack(7, 0.0, 0.0, at);
    (a.state, a.seen, a.sub_action) = (KILLED, at, true);
    assert_eq!(a.update(walk, &mut s), Ok(5));
    assert_eq!((a.state, a.target), (DONE, -1));
    assert_eq!(s.posted, vec![(0x1fb, [600, 0, 0x202])]);
    assert_eq!(mover.animated.speed, 1.0);
    assert_eq!(s.let_go, None);
}

/// A dead tree (class 15) up to state 5 keeps a non-villager's attack going; ready, the
/// attack on it ends (`set_state` 1) and returns 5. A villager's, or one on a tree past
/// state 5, ends at the update's start, letting the target go.
#[test]
fn a_dead_tree_keeps_a_soldier_s_attack_until_it_is_ready() {
    let at = [11.0, 10.0, 0.0];
    let run = |unit_class: i16, tree_state: u8| {
        let mut world = world_with(7, 0, TREE, at);
        let mut mover = soldier_mover(0.8);
        let walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            class: TREE,
            state: tree_state,
            position: at,
            ..Victim::default()
        });
        let mut s = rig.strike();
        s.own.class = unit_class;
        let mut a = soldier_attack(7, 1.0, 0.0, at);
        a.seen = at;
        let code = a.update(walk, &mut s).unwrap();
        (code, a.state, s.let_go)
    };
    assert_eq!(run(6, 3), (5, DONE, None));
    assert_eq!(run(6, 5), (5, DONE, None));
    assert_eq!(run(4, 3), (5, DONE, Some(7)));
    assert_eq!(run(6, 6), (5, DONE, Some(7)));
}

/// After a hit, the swing's loop test reads the unit's sprite as readyToAttack left it: one
/// that failed (`set_state` 13, the standing sprite from its first frame) keeps the attack in
/// state 13, though the attack sprite had looped.
#[test]
fn a_swing_that_fails_its_interrupt_check_stays_failed() {
    let at = [15.0, 10.0, 0.0];
    let mut world = world_with(7, 0, 6, at);
    let mut mover = soldier_mover(0.0);
    let walk = walk_in(&mut mover, &mut world);
    let mut rig = Rig::new(Victim {
        id: 7,
        state: 2,
        position: at,
        ..Victim::default()
    });
    let swing = Graphic {
        id: 301,
        ..STANDING
    };
    rig.sprite = Some(ActiveSprite {
        looped: true,
        frame: 9,
        ..ActiveSprite::start(swing, &mut Well512::default())
    });
    let mut s = rig.strike();
    let mut a = soldier_attack(7, 1.0, 0.0, [NONE; 3]);
    (a.state, a.seen, a.arms.interrupt_frame) = (SWING, at, 5);
    assert_eq!(a.update(walk, &mut s), Ok(0));
    assert_eq!(a.state, FAILED);
    assert_eq!(s.posted, vec![(0x1fd, [0; 3])]);
    assert_eq!(s.sprite.map(|sp| sp.graphic.id), Some(STANDING.id));
}

#[test]
fn a_rangeless_attack_reaches_a_tenth_squared_in_f32() {
    // the image holds 0x3c23d70b (0.1f * 0.1f), one ULP above 0.01f
    assert_eq!(squared_reach(0.0).to_bits(), 0x3c23_d70b);
    assert_eq!(squared_reach(NO_RANGE).to_bits(), 0x3c23_d70b);
    assert_eq!(squared_reach(2.0), 4.0);
}

/// 425906646's town centre 27171 (player 6) in update 6224: its Attack lost the boar 29826
/// and the town centre cannot attack (no missile, a firepower of at most 1): the Attack ends
/// (`set_state(1)`, the standing sprite at speed 1) and posts 0x1fb with 600 and its target,
/// −1.
#[test]
fn a_disarmed_building_s_targetless_attack_ends_with_its_notify() {
    let standing = Graphic {
        id: 418,
        frames: 1,
        facets: 1,
        speed_multiplier: 1.0,
        frame_duration: 0.0,
        replay_delay: 0.0,
        flags: 0,
    };
    let mut a = Attack {
        target: -1,
        target2: -1,
        state: READY,
        timer: 0.0,
        range: 6.0,
        min_range: 0.0,
        point: [NONE; 3],
        seen: [NONE; 3],
        goto: [NONE; 3],
        move_to: None,
        move_wait: MoveWait::default(),
        pending: false,
        swing: 0.0,
        rest: 0.0,
        arms: Arms {
            move_sprite: None,
            sprite: None,
            second_sprite: None,
            idle_sprite: Some(standing),
            idle_attack_sprite: None,
            missile: -1,
            fire_frame: 0,
            interrupt_frame: -1,
        },
        sub_action: false,
        repaths: false,
        runs: false,
        same_owner: false,
    };
    let mut body = Body::Animated(Animated { speed: 0.5 });
    assert_eq!(
        a.clone().update_targetless(0.028, true, &mut body),
        Err(GenieError::unsupported(
            "an Attack without a target in a state but 1"
        ))
    );
    assert_eq!(a.update_targetless(0.028, false, &mut body), Ok((1, true)));
    assert_eq!(a.state, DONE);
    let (mut sprite, mut rng) = (None, Well512::default());
    let posted = a.disarmed(&mut sprite, &mut body, &mut rng).unwrap();
    assert_eq!(posted, Some((0x1fb, [600, -1, 0])));
    assert_eq!(
        (body.anim_speed(), sprite.map(|s| s.graphic.id)),
        (Some(1.0), Some(418))
    );
    // the Hunt's Attack, a sub-action, posts nothing
    a.sub_action = true;
    assert_eq!(a.disarmed(&mut sprite, &mut body, &mut rng).unwrap(), None);
}

/// `FUN_140a7c210`: without a `MoveTo` sprite of its own the Attack walks with the unit's
/// running sprite, else its walking one, slots 0x7d0 and 0x7c8: a terrain look's while the
/// unit has one; the attack sprite stays the master's.
#[test]
fn an_attack_walks_with_the_unit_s_running_or_walking_slot() {
    let g = |id| Graphic {
        id,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let graphics: std::collections::BTreeMap<i16, Graphic> = [10, 11, 12, 20, 21]
        .into_iter()
        .map(|id| (id, g(id)))
        .collect();
    let master = crate::objects::masters::Master {
        move_sprite: 10,
        run_sprite: 11,
        attack_sprite: 12,
        ..crate::objects::masters::Master::default()
    };
    let a = arms(&master, &graphics, [-1; 4], -1);
    assert_eq!(a.move_sprite.map(|s| s.id), Some(11));
    let look = crate::combat::abilities::Look {
        standing: None,
        walking: Some(20),
        attacking: Some(21),
        running: None,
        dying: None,
        undead: None,
    };
    let a = arms_looking(&master, Some(&look), &graphics, [-1; 4], -1);
    assert_eq!(a.move_sprite.map(|s| s.id), Some(20));
    assert_eq!(a.sprite.map(|s| s.id), Some(12));
    // its own MoveTo sprite first
    let a = arms_looking(&master, Some(&look), &graphics, [-1; 4], 10);
    assert_eq!(a.move_sprite.map(|s| s.id), Some(10));
}

/// The soldier at (10, 10), master 4, of speed `speed` whose unit AI may move to attack
/// (`can_move`) in stance `stance`, its leash centre on itself, and a target of class 6 `gap`
/// east of it; range 1, no least range.
fn engaged(gap: f32, speed: f32, can_move: bool, stance: u8) -> Engagement {
    Engagement {
        gap_squared: gap * gap,
        gap,
        position: [10.0, 10.0, 0.0],
        target_position: [10.0 + gap, 10.0, 0.0],
        speed,
        range: 1.0,
        min_range: 0.0,
        can_move,
        stance,
        state_position: [10.0, 10.0],
        world_time: 0,
        master: 4,
        target_class: 6,
        same_owner: false,
    }
}

fn standing_arms() -> Arms {
    Arms {
        idle_sprite: Some(STANDING),
        ..own_attack(0, 0, 0.0).arms
    }
}

/// The attack command's start of a unit that cannot move (no top speed, or a unit AI in
/// stance 2 without an order, 426049998's scout 1222): readyToAttack's ready end in reach
/// (the target's place the point, a NaN gap within) is `set_state(6)`; out of reach (0x1fd)
/// or below the least range (0x1fe with −1s) it fails (`set_state(13)`).
#[test]
fn the_start_of_a_unit_that_cannot_move_goes_ready_in_reach_and_fails_out_of_it() {
    let start = |e: Engagement| {
        let mut mover = soldier_mover(e.speed);
        let world = world_with(7, 2, 6, e.target_position);
        Attack::command((7, &world), &e, standing_arms(), (true, 2), &mut mover).unwrap()
    };
    for e in [
        engaged(0.5, 1.2, false, 2),
        engaged(0.5, 0.0, true, 0),
        engaged(f32::NAN, 1.2, false, 2),
    ] {
        let c = start(e);
        assert_eq!(
            (c.ready, c.failed, c.failed_first, c.arrived, c.posted),
            (true, false, false, false, vec![])
        );
        assert_eq!(
            c.attack.point.map(f32::to_bits),
            e.target_position.map(f32::to_bits)
        );
        assert_eq!(c.attack.move_to, None);
    }
    let away = start(engaged(3.0, 1.2, false, 2));
    assert_eq!(
        (away.ready, away.failed, away.posted),
        (false, true, vec![(0x1fd, [0; 3])])
    );
    assert_eq!(away.attack.point, [NONE; 3]);
    let close = start(Engagement {
        min_range: 1.0,
        ..engaged(0.5, 1.2, false, 2)
    });
    assert_eq!(
        (close.ready, close.failed, close.posted),
        (false, true, vec![(0x1fe, [-1; 3])])
    );
}

/// The start of a unit that may move ends in `set_state(4)` whatever readyToAttack did: below
/// the least range after its 0x1fe; out of reach after its own `set_state(4)` (whose `MoveTo`
/// the second one's teardown stops); on a target out of sight after its `set_state(5)` (stance
/// 0, its `MoveTo` stopped too) or its `set_state(13)` (another stance), which leaves the
/// animation at speed 1; stance 1's leash fails a target 5 past the range from the unit AI's
/// point and holds a nearer one.
#[test]
fn the_start_of_a_unit_that_may_move_turns_to_state_4_whatever_ready_to_attack_found() {
    let start = |e: Engagement, visible: bool| {
        let mut mover = soldier_mover(e.speed);
        mover.animated.speed = 1.0;
        let world = world_with(7, 2, 6, e.target_position);
        let c =
            Attack::command((7, &world), &e, standing_arms(), (visible, 2), &mut mover).unwrap();
        assert_eq!((c.attack.state, c.ready, c.failed), (MOVE, false, false));
        assert_eq!(c.attack.move_to, Some(MoveTo::Start));
        (c.failed_first, c.posted, mover.animated.speed == 0.0)
    };
    assert_eq!(
        start(engaged(0.5, 1.2, true, 0), true),
        (false, vec![], false)
    );
    let close = Engagement {
        min_range: 1.0,
        ..engaged(0.5, 1.2, true, 0)
    };
    assert_eq!(start(close, true), (false, vec![(0x1fe, [-1; 3])], false));
    assert_eq!(
        start(engaged(3.0, 1.2, true, 0), true),
        (false, vec![(0x1fd, [0; 3])], true)
    );
    assert_eq!(
        start(engaged(3.0, 1.2, true, 0), false),
        (false, vec![], true)
    );
    assert_eq!(
        start(engaged(3.0, 1.2, true, 2), false),
        (true, vec![], false)
    );
    // stance 1: the target 3 from the centre stays within 1 + 5, 7 from it does not
    assert_eq!(
        start(engaged(3.0, 1.2, true, 1), true),
        (false, vec![(0x1fd, [0; 3])], true)
    );
    let far = Engagement {
        state_position: [3.0, 10.0],
        ..engaged(3.0, 1.2, true, 1)
    };
    assert_eq!(start(far, true), (true, vec![(0x1fd, [0; 3])], false));
    // a target out of sight of a unit that cannot follow it fails
    let mut mover = soldier_mover(1.2);
    let e = engaged(3.0, 1.2, false, 2);
    let world = world_with(7, 2, 6, e.target_position);
    let c = Attack::command((7, &world), &e, standing_arms(), (false, 2), &mut mover).unwrap();
    assert_eq!((c.failed, c.ready, c.posted), (true, false, vec![]));
}

/// readyToAttack's ready end turns the unit to its target (`FUN_141162db0`) but for masters
/// 1704 and 1706.
#[test]
fn the_start_faces_the_target_but_for_masters_1704_and_1706() {
    let north = Engagement {
        target_position: [10.0, 10.5, 0.0],
        ..engaged(0.5, 0.0, false, 2)
    };
    let world = world_with(7, 2, 6, north.target_position);
    let angle = |master| {
        let mut mover = soldier_mover(0.0);
        mover.angle = 0.0;
        let e = Engagement { master, ..north };
        let _ = Attack::command((7, &world), &e, standing_arms(), (true, 2), &mut mover).unwrap();
        mover.angle
    };
    assert_ne!(angle(4), 0.0);
    assert_eq!(angle(1704), 0.0);
    assert_eq!(angle(1706), 0.0);
}

/// A gate's reach (class 39): the nearest of it and its first two parts (slot 0x680 in id
/// order, `MINSS`), in the start's readyToAttack and an update's; the least range reads the
/// gate's own gap. A gate 3 tiles off with a part 0.5 off is in reach of a range of 1.
#[test]
fn a_gate_s_reach_is_to_its_nearest_part() {
    let gate_at = [13.4, 10.0, 0.0];
    let gated = || {
        let mut world = world_with(7, 0, GATE, gate_at);
        for (id, x) in [(8, 10.9), (9, 20.0), (10, 10.5)] {
            world.insert(
                id,
                crate::map::spatial::Placed {
                    position: [x, 10.0, 0.0],
                    ..*world.get(7).unwrap()
                },
            );
            world.set_annex(id, Some(7));
        }
        world
    };
    let world = gated();
    let e = Engagement {
        target_class: GATE,
        ..engaged(3.0, 0.0, false, 2)
    };
    let mut mover = soldier_mover(0.0);
    let c = Attack::command((7, &world), &e, standing_arms(), (true, 2), &mut mover).unwrap();
    // the part 8 is 0.5 off (its footprint 0.1); the part 10, the third, is not read
    assert_eq!((c.ready, c.failed, c.posted), (true, false, vec![]));
    // without its parts the gate is out of reach
    let bare = world_with(7, 0, GATE, gate_at);
    let mut mover = soldier_mover(0.0);
    let c = Attack::command((7, &bare), &e, standing_arms(), (true, 2), &mut mover).unwrap();
    assert_eq!((c.ready, c.failed), (false, true));
    // an update's readyToAttack the same way
    let mut world = gated();
    let mut mover = soldier_mover(0.0);
    let mut walk = walk_in(&mut mover, &mut world);
    let mut rig = Rig::new(Victim {
        id: 7,
        position: gate_at,
        ..Victim::default()
    });
    let mut s = rig.strike();
    s.can_move = false;
    let mut a = soldier_attack(7, 1.0, 0.0, [NONE; 3]);
    let r = a.ready_to_attack(&mut walk, &mut s, false).unwrap();
    assert!(matches!(r, Readiness::Ready));
    assert_eq!(a.point, gate_at);
}

/// `set_state(6)` at the start of a unit that cannot move: readyToAttack again finds the
/// target on the point; facing it with its attack timer run out the unit swings
/// (`set_state(7)`: the attack sprite, a hit pending), else it waits in state 6. Without a
/// target or a point a unit's own Attack posts 0x1fa with 600 and ends (`set_state(1)`).
#[test]
fn the_ready_start_swings_at_once_when_its_timer_ran_out() {
    let at = [10.5, 10.0, 0.0];
    let attack_sprite = Graphic {
        id: 301,
        ..STANDING
    };
    let ready = |timer: f32| {
        let mut world = world_with(7, 0, 6, at);
        let mut mover = soldier_mover(0.0);
        let walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            position: at,
            ..Victim::default()
        });
        rig.timer = timer;
        let mut s = rig.strike();
        s.can_move = false;
        let mut a = soldier_attack(7, 1.0, 0.0, at);
        a.arms.sprite = Some(attack_sprite);
        a.state = 0;
        a.enter_ready(walk, &mut s).unwrap();
        assert_eq!(s.posted, vec![]);
        (a.state, a.pending, rig.sprite.map(|sp| sp.graphic.id))
    };
    assert_eq!(ready(0.0), (SWING, true, Some(301)));
    assert_eq!(ready(0.5), (READY, false, None));
    let mut world = world_with(7, 0, 6, at);
    let mut mover = soldier_mover(0.0);
    let walk = walk_in(&mut mover, &mut world);
    let mut rig = Rig::new(Victim::default());
    let mut s = rig.strike();
    let mut a = soldier_attack(-1, 1.0, 0.0, [NONE; 3]);
    a.enter_ready(walk, &mut s).unwrap();
    assert_eq!(a.state, DONE);
    assert_eq!(s.posted, vec![(0x1fa, [600, 0, 0])]);
}

/// `set_state(7)`'s sprite: a second attack sprite makes the swing draw the Object stream
/// (`FUN_140810fc0` with 2: 0 the first sprite, 1 the second); the terrain look's attack
/// sprite comes first and draws nothing, and so does a special graphic on a charge that
/// hits.
#[test]
fn a_swing_with_a_second_sprite_draws_which_one_it_shows() {
    let at = [10.5, 10.0, 0.0];
    let sprite = |id: i16| Graphic { id, ..STANDING };
    let swing = |object: Well512, sprites: SwingSprites| {
        let mut object = object;
        let mut world = world_with(7, 0, 6, at);
        let mut mover = soldier_mover(0.0);
        let mut pathing = crate::pathing::Pathing::default();
        let groups = crate::groups::UnitGroups::default();
        let mut walk = walk_in(&mut mover, &mut world);
        walk.plan = Some(crate::movement::Planning {
            pathing: &mut pathing,
            now: 0,
            computer: false,
            object: &mut object,
            construction_rate: 1.0,
            groups: &groups,
        });
        let mut rig = Rig::new(Victim {
            id: 7,
            position: at,
            ..Victim::default()
        });
        let mut s = rig.strike();
        s.can_move = false;
        s.swing_sprites = sprites;
        let mut a = soldier_attack(7, 1.0, 0.0, at);
        a.arms.sprite = Some(sprite(301));
        a.arms.second_sprite = Some(sprite(302));
        a.state = 0;
        a.enter_ready(walk, &mut s).unwrap();
        (rig.sprite.map(|sp| sp.graphic.id), object)
    };
    // two seeds whose first draw scales to 0 and to 1
    let seeded = |seed: u32| {
        let mut r = Well512::default();
        r.set_seed(seed);
        r
    };
    let first = |seed: u32| seeded(seed).below_checked(2);
    let (zero, one) = (
        (0..64).find(|&s| first(s) == 0).unwrap(),
        (0..64).find(|&s| first(s) == 1).unwrap(),
    );
    let (shown, left) = swing(seeded(zero), SwingSprites::default());
    let mut drawn = seeded(zero);
    let _ = drawn.below_checked(2);
    assert_eq!((shown, left), (Some(301), drawn));
    assert_eq!(swing(seeded(one), SwingSprites::default()).0, Some(302));
    // the look's sprite draws nothing
    let looked = SwingSprites {
        look: Some(sprite(303)),
        special: None,
    };
    assert_eq!(swing(seeded(one), looked), (Some(303), seeded(one)));
    // a special graphic without a charge that hits draws as before
    let special = SwingSprites {
        look: None,
        special: Some(sprite(304)),
    };
    assert_eq!(swing(seeded(one), special).0, Some(302));
}

/// readyToAttack's leash in an update: a unit in stance 1 whose target stands the range
/// plus 5 or more from its unit AI's `state_position` fails (`set_state(13)`) after its
/// 0x1fd; a nearer one follows. A NaN distance holds (`COMISS`/`JNC`).
#[test]
fn ready_to_attack_s_leash_fails_a_target_too_far_from_the_unit_ai_s_point() {
    let at = [15.0, 10.0, 0.0];
    for (centre, follows) in [
        ([10.0, 10.0], true),
        ([9.0, 10.0], false),
        ([f32::NAN, 10.0], true),
    ] {
        let mut world = world_with(7, 0, 6, at);
        let mut mover = soldier_mover(0.8);
        let mut walk = walk_in(&mut mover, &mut world);
        let mut rig = Rig::new(Victim {
            id: 7,
            position: at,
            ..Victim::default()
        });
        let mut s = rig.strike();
        (s.stance, s.state_position) = (1, centre);
        let mut a = soldier_attack(7, 1.0, 0.0, [NONE; 3]);
        let r = a.ready_to_attack(&mut walk, &mut s, false).unwrap();
        assert_eq!(s.posted, vec![(0x1fd, [0; 3])]);
        assert_eq!(matches!(r, Readiness::Away(_)), follows, "{centre:?}");
        assert_eq!(a.state, if follows { READY } else { FAILED });
    }
}

/// The unit's own Attack in its update without a target ([`Unit::attack_part`]): a building
/// that cannot attack (`armed` false) ends it with its standing sprite at speed 1, posts 0x1fb
/// with 600 and no target, and the Attack goes (code 1). An Attack whose target the world does
/// not hold live is not modelled.
///
/// [`Unit::attack_part`]: crate::objects::unit::Unit::attack_part
#[test]
fn a_disarmed_building_s_targetless_attack_ends_standing_and_posts_0x1fb() {
    use crate::world::tests::{forage_world, in_update};
    let stand = Graphic {
        id: 9,
        frames: 10,
        facets: 1,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let me = |id, position| crate::movement::Me {
        id,
        position,
        group: -1,
        action_state: 0,
    };
    let mut t = forage_world();
    // the town centre's master without a missile cannot attack
    for masters in &mut t.masters {
        masters.get_mut(109).unwrap().missile = -1;
    }
    let centre = &mut t.players[1].objects[2];
    let mut attack = own_attack(-1, 1, 0.0);
    attack.arms.idle_sprite = Some(stand);
    centre.actions.set_attack(Some(attack));
    // its unit AI on the attack's action, which takes the Attack's invalidation
    centre.ai = Some(crate::unit_ai::UnitAi {
        current_action: 600,
        ..crate::unit_ai::UnitAi::default()
    });
    let at = centre.position;
    in_update(&mut t, (1, 2), 50, |u, c| {
        let code = u.attack_part(c, me(700, at), None)?;
        u.end_head_update(c, code)
    })
    .unwrap();
    let centre = &t.players[1].objects[2];
    assert_eq!(centre.actions.attack().copied(), None);
    assert_eq!(
        (
            centre.sprite.map(|s| s.graphic.id),
            centre.body.anim_speed()
        ),
        (Some(9), Some(1.0))
    );
    let posted = &centre.ai.as_ref().unwrap().notifies;
    assert!(
        posted
            .iter()
            .any(|n| n.kind == 0x1fb && n.p == [600, -1, 0])
    );
    let mut t = forage_world();
    t.players[1].objects[0]
        .actions
        .set_attack(Some(own_attack(77, 1, 0.0)));
    let at = t.players[1].objects[0].position;
    let r = in_update(&mut t, (1, 0), 50, |u, c| {
        let code = u.attack_part(c, me(9175, at), None)?;
        u.end_head_update(c, code)
    });
    assert_eq!(
        r.unwrap_err().reason(),
        "an attack's target the slice does not hold live"
    );
}

/// A sprite of [`STANDING`]'s graphic (ten frames of 0.1 s) that just advanced from frame
/// `last` to `frame`.
fn at_frame(frame: i16, last: i16) -> ActiveSprite {
    ActiveSprite {
        frame,
        last_frame: last,
        changed: true,
        looped: false,
        ..ActiveSprite::start(STANDING, &mut Well512::default())
    }
}

/// [`own_attack`] on the house 10028 in state `state`, its swing pending, with the standing
/// sprite to end with and a missile (`ranged`) firing at frame `fire`.
fn swinging_attack(state: u8, ranged: bool, fire: i16) -> Attack {
    let mut a = own_attack(10028, state, 0.1);
    a.pending = true;
    a.arms.idle_sprite = Some(STANDING);
    a.arms.missile = if ranged { 5 } else { -1 };
    a.arms.fire_frame = fire;
    a
}

/// The swing before its hit ([`Attack::swing_to_hit`]): a ranged swing waits for its firing
/// frame without counting down, then fires its volley at its target ([`Attack::land_hit`]): the
/// attack timer is the reload less the time to that frame, the rest of the reload past the
/// sprite the swing's; a melee swing counts down by the walk's dt toward half its frames.
#[test]
fn a_ranged_swing_fires_at_its_firing_frame_and_a_melee_one_counts_down() {
    use crate::movement::tests::WalkRig;
    let mut walker = WalkRig::on_the_path();
    let mut rig = Rig::new(Victim {
        hp: 50.0,
        ..Victim::default()
    });
    let mut s = rig.strike();
    let mut a = swinging_attack(SWING, true, 3);
    a.swing = 1.0;
    a.swing_to_hit(&mut walker.walk(7), &mut s, at_frame(2, 1), true)
        .unwrap();
    assert_eq!((a.pending, s.fire, a.swing), (true, None, 1.0));
    a.swing_to_hit(&mut walker.walk(7), &mut s, at_frame(3, 2), true)
        .unwrap();
    assert_eq!((a.pending, s.fire), (false, Some(10028)));
    assert_eq!(*s.attack_timer, 2.0 - 3.0 * STANDING.frame_duration);
    let rest = 2.0 - 10.0 * STANDING.frame_duration;
    assert_eq!((a.rest, a.swing), (rest, rest));
    let mut melee = swinging_attack(SWING, false, 0);
    melee.swing = 1.0;
    melee
        .swing_to_hit(&mut walker.walk(7), &mut s, at_frame(4, 3), false)
        .unwrap();
    assert_eq!((melee.pending, melee.swing), (true, 1.0 - 0.1));
}

/// The swing's unmodelled parts: class 51's swing, a melee swing of class 35, a ranged one
/// without a firing frame and a self-destructing attacker's melee hit; and a ranged hit that
/// kills its target ends the swing (`set_state` 2).
#[test]
fn a_swing_s_unmodelled_parts_fail_and_a_killing_hit_ends_it() {
    use crate::movement::tests::WalkRig;
    let reason = |class, master, ranged, fire| {
        let mut walker = WalkRig::on_the_path();
        let mut rig = Rig::new(Victim {
            hp: 50.0,
            ..Victim::default()
        });
        let mut s = rig.strike();
        (s.own.class, s.own.master_id) = (class, master);
        let mut a = swinging_attack(SWING, ranged, fire);
        a.swing_to_hit(&mut walker.walk(7), &mut s, at_frame(5, 4), ranged)
            .unwrap_err()
            .reason()
    };
    assert_eq!(reason(51, 4, true, 3), "a swing of class 35, 51 or 64");
    assert_eq!(reason(35, 4, false, 0), "a swing of class 35, 51 or 64");
    assert_eq!(
        reason(6, 4, true, -1),
        "a ranged swing without a firing frame"
    );
    assert_eq!(
        reason(6, 527, false, 0),
        "a self-destructing attacker's hit (masters 527, 528, 1104, 2133..2135)"
    );
    let mut walker = WalkRig::on_the_path();
    let mut rig = Rig::new(Victim::default());
    let mut s = rig.strike();
    let mut a = swinging_attack(SWING, true, 3);
    a.land_hit(&mut walker.walk(7), &mut s, at_frame(3, 2), true)
        .unwrap();
    assert_eq!((a.pending, a.state, s.fire), (false, KILLED, Some(10028)));
}

/// The swing after its hit ([`Attack::swing_after_hit`]): a ranged swing whose timer ran out
/// without a target ends (`set_state` 2), with one past its interrupt frame readies
/// (`set_state` 6, whose readyToAttack finds the house out of sight: the unit goes to where it
/// saw it, state 5); one going on waits for its sprite to loop; a looped sprite with reload
/// left shows the idle sprite at speed 1 and readies.
#[test]
fn a_swing_after_its_hit_ends_readies_or_waits_for_its_sprite() {
    use crate::movement::tests::WalkRig;
    let after = |target, interrupt, sprite: ActiveSprite, swing| {
        let mut walker = WalkRig::on_the_path();
        let mut rig = Rig::new(Victim::default());
        let mut s = rig.strike();
        let mut a = swinging_attack(SWING, true, 3);
        (a.pending, a.target, a.swing) = (false, target, swing);
        a.arms.interrupt_frame = interrupt;
        a.swing_after_hit(&mut walker.walk(7), &mut s, sprite, true)
            .unwrap();
        (a.state, walker.mover.animated.speed)
    };
    assert_eq!(after(-1, 5, at_frame(9, 8), 0.0).0, KILLED);
    assert_eq!(after(10028, 5, at_frame(6, 5), 0.0).0, GOTO_SEEN);
    assert_eq!(after(10028, -1, at_frame(3, 2), 0.0).0, SWING);
    let looped = ActiveSprite {
        looped: true,
        ..at_frame(0, 9)
    };
    assert_eq!(after(10028, -1, looped, 0.5), (GOTO_SEEN, 1.0));
}

/// State 4's `MoveTo` start ([`Attack::walk_start`]) on the house 10028: after a wait `+0x6c`
/// goes from 1 to 2, a running `MoveTo` takes the running speed and shows its move sprite as it
/// moves; one in range at once arrives and shows the idle sprite with `+0x7b8` cleared.
#[test]
fn an_attack_s_move_to_starts_running_or_arrives_in_range() {
    use crate::movement::tests::WalkRig;
    let walking = Graphic {
        id: 301,
        ..STANDING
    };
    let start = |range| {
        let mut walker = WalkRig::on_the_path();
        walker.mover.stats.run_speed = 1.2;
        let mut rig = Rig::new(Victim::default());
        let mut s = rig.strike();
        walker.mover.animated.speed = 1.0;
        let mut a = own_attack(10028, MOVE, range);
        (a.move_to, a.move_wait.state) = (Some(MoveTo::Start), 1);
        (a.arms.move_sprite, a.arms.idle_sprite) = (Some(walking), Some(STANDING));
        let walked = a.walk_start(&mut walker.walk(4), &mut s).unwrap();
        let shown = s.sprite.map(|g| g.graphic.id);
        (
            walked,
            a.move_to,
            a.move_wait.state,
            shown,
            walker.mover.animated.speed,
            walker.mover.stats.speed,
        )
    };
    assert_eq!(
        start(0.1),
        (true, Some(MoveTo::Moving), 2, Some(301), 1.0, 1.2)
    );
    let (walked, move_to, _, shown, anim, _) = start(5.0);
    assert_eq!(
        (walked, move_to, shown, anim),
        (true, Some(MoveTo::Arrived), Some(300), 0.0)
    );
}

/// State 4's `MoveTo` walking ([`Attack::walk_on`]) on the house 10028: a target that stays is
/// walked to; one that left the `MoveTo`'s point only turns it to state 3 with `+0x61`, and
/// without it tells the unit (notify 0x1fc) and runs the motion step first.
#[test]
fn an_attack_s_move_to_follows_a_target_that_left_its_point() {
    use crate::movement::tests::WalkRig;
    let walk_on = |point: [f32; 3], repaths| {
        let mut walker = WalkRig::on_the_path();
        walker.mover.move_to_point = Some(point);
        walker.mover.paths[0].waypoint = 0;
        let mut rig = Rig::new(Victim::default());
        let mut s = rig.strike();
        let mut a = own_attack(10028, MOVE, 0.1);
        (a.move_to, a.repaths) = (Some(MoveTo::Moving), repaths);
        a.walk_on(&mut walker.walk(4), &mut s).unwrap();
        (a.move_to, s.posted.clone())
    };
    assert_eq!(
        walk_on([30.0, 88.0, 0.0], false),
        (Some(MoveTo::Moving), vec![])
    );
    assert_eq!(
        walk_on([36.0, 88.0, 0.0], true),
        (Some(MoveTo::Start), vec![])
    );
    assert_eq!(
        walk_on([36.0, 88.0, 0.0], false),
        (Some(MoveTo::Start), vec![(0x1fc, [0; 3])])
    );
}

/// State 4's update ([`Attack::walk`]) on the house 10028 out of reach: a walking `MoveTo`
/// walks on; a wait runs by the walk's dt and asks again at its delay; an arrived `MoveTo` goes
/// and stops the unit, which readies (the house out of sight: it goes to where it saw it); a
/// failed one ends the Attack (code 3), a unit's own posting 0x1f9 with 600. An Attack in
/// state 4 without its `MoveTo` is not modelled.
#[test]
fn an_attack_s_walk_walks_waits_arrives_or_fails() {
    use crate::movement::tests::WalkRig;
    let walked = |move_to: Option<MoveTo>, timer: f32| {
        let mut walker = WalkRig::on_the_path();
        let mut rig = Rig::new(Victim::default());
        let mut s = rig.strike();
        let mut a = own_attack(10028, MOVE, 0.1);
        a.arms.idle_sprite = Some(STANDING);
        a.move_to = move_to;
        a.move_wait = MoveWait {
            state: 1,
            timer,
            delay: 0.15,
        };
        let code = a
            .walk(walker.walk(4), &mut s, [30.0, 88.0, 0.0], [1.0, 1.0])
            .map_err(|e| e.reason());
        let posted = s.posted.clone();
        (code, a, posted, walker.mover.animated.speed == 0.0)
    };
    let (code, a, posted, _) = walked(Some(MoveTo::Moving), 0.0);
    assert_eq!(
        (code, a.move_to, a.state, posted),
        (Ok(0), Some(MoveTo::Moving), MOVE, vec![])
    );
    let (_, a, ..) = walked(Some(MoveTo::Waiting), 0.0);
    assert_eq!((a.move_to, a.move_wait.timer), (Some(MoveTo::Waiting), 0.1));
    let (_, a, ..) = walked(Some(MoveTo::Waiting), 0.1);
    assert_eq!((a.move_to, a.move_wait.timer), (Some(MoveTo::Start), 0.0));
    let (code, a, _, stopped) = walked(Some(MoveTo::Arrived), 0.0);
    assert_eq!((code, a.state, stopped), (Ok(0), GOTO_SEEN, true));
    let (code, a, posted, _) = walked(Some(MoveTo::Failed), 0.0);
    assert_eq!(
        (code, a.state, posted),
        (
            Ok(3),
            FAILED,
            vec![(crate::unit_ai::ACTION_FAILED, [600, 0, 0])]
        )
    );
    assert_eq!(
        walked(None, 0.0).0,
        Err("an Attack in state 4 without its MoveTo")
    );
}

/// An Attack of the soldier at (10, 10) on target 7 (a victim of `player` and `class` in
/// `state` at `at`, `seen` or not by the unit's player), in `attack_state`, its sub-action
/// `sub` or its own; `f` runs on the Attack, its walk and its strike.
fn on_target<R>(
    (player, class, state, at): (usize, i16, u8, [f32; 3]),
    seen: bool,
    (attack_state, sub): (u8, bool),
    f: impl FnOnce(&mut Attack, &mut Walk<'_>, &mut Strike<'_>) -> R,
) -> (R, Attack) {
    let mut world = world_with(7, player, class, at);
    world.sight = Some(sight(seen));
    let mut mover = soldier_mover(0.8);
    let mut walk = walk_in(&mut mover, &mut world);
    let mut rig = Rig::new(Victim {
        id: 7,
        state,
        class,
        player,
        position: at,
        ..Victim::default()
    });
    let mut s = rig.strike();
    s.ai_order = Some(Some(OrderKind::Attack));
    let mut a = soldier_attack(7, 1.0, 0.0, at);
    (a.state, a.seen, a.sub_action) = (attack_state, at, sub);
    let r = f(&mut a, &mut walk, &mut s);
    (r, a)
}

/// The update's start on a dead target: a unit's own Attack lets it go and ends with 5; a
/// villager's sub-action on Gaia's target forgets it and its point and ends with 5; another
/// unit's sub-action, or a villager's on a player's target, is not modelled.
#[test]
fn an_attack_whose_target_died_lets_it_go_and_ends() {
    let dead = |player| (player, 6, 3, [11.0, 10.0, 0.0]);
    let ((code, let_go), a) = on_target(dead(0), true, (READY, false), |a, w, s| {
        s.ai_order = Some(None);
        (a.target_died(w, s).unwrap(), s.let_go)
    });
    assert_eq!((code, let_go, a.state), (5, Some(7), DONE));
    let (code, a) = on_target(dead(0), true, (READY, true), |a, w, s| {
        s.own.class = VILLAGER;
        a.target_died(w, s).unwrap()
    });
    assert_eq!((code, a.target, a.point, a.state), (5, -1, [NONE; 3], DONE));
    let (r, _) = on_target(dead(0), true, (READY, true), |a, w, s| {
        a.target_died(w, s).unwrap_err().reason()
    });
    assert_eq!(
        r,
        "an Attack of a unit other than a villager whose target died"
    );
    let (r, _) = on_target(dead(2), true, (READY, true), |a, w, s| {
        s.own.class = VILLAGER;
        a.target_died(w, s).unwrap_err().reason()
    });
    assert_eq!(
        r,
        "a villager's Attack whose player-owned target died (its unit AI forgets it)"
    );
}

/// A target that moved: where it was seen follows; seen, an Attack walking to where it saw
/// it follows it (`set_state(4)`); out of sight, Gaia's unit fails, a player's unit with an
/// order and an empty queue waits from state 4 and walks on in state 5; seen in state 4 it
/// goes on.
#[test]
fn an_attack_on_a_moved_target_follows_waits_or_fails() {
    let moved = (2, 6, 2, [13.0, 10.0, 0.0]);
    let run = |state: u8, seen: bool, own_player: usize| {
        on_target(moved, seen, (state, false), |a, w, s| {
            a.seen = [12.0, 10.0, 0.0];
            s.own.player = own_player;
            let t = *w.world.get(7).unwrap();
            a.target_moved(w, s, &t).unwrap()
        })
    };
    let (code, a) = run(GOTO_SEEN, true, 1);
    assert_eq!((code, a.state, a.seen), (Some(0), MOVE, [13.0, 10.0, 0.0]));
    let (code, a) = run(MOVE, true, 1);
    assert_eq!((code, a.state, a.seen), (None, MOVE, [13.0, 10.0, 0.0]));
    let (code, a) = run(MOVE, false, 0);
    assert_eq!((code, a.state), (Some(3), FAILED));
    let (code, a) = run(MOVE, false, 1);
    assert_eq!((code, a.state), (Some(0), GOTO_SEEN));
    let (code, a) = run(GOTO_SEEN, false, 1);
    assert_eq!((code, a.state), (None, GOTO_SEEN));
}

/// The update's ends before the state's step: a live target the unit's player counts as its
/// own ends it with 1 (a unit's own Attack posting 0x1fb); a target inside another ends it
/// with 4; a building that cannot attack ends it with 1; else the state's step follows.
#[test]
fn an_attack_ends_early_on_an_ally_a_hidden_target_or_disarmed() {
    let live = (2, 6, 2, [11.0, 10.0, 0.0]);
    let run = |f: fn(&mut Strike<'_>)| {
        on_target(live, true, (MOVE, false), |a, w, s| {
            f(s);
            (a.ends_early(w, s, true).unwrap(), s.posted.clone())
        })
    };
    let ((code, posted), a) = run(|s| s.victim.allied[0] = true);
    assert_eq!(
        (code, a.state, posted),
        (Some(1), DONE, vec![(0x1fb, [600, 7, 0])])
    );
    let ((code, posted), a) = run(|s| s.victim.inside = true);
    assert_eq!(
        (code, a.state, posted),
        (Some(4), DONE, vec![(0x1fb, [600, 7, 0])])
    );
    let ((code, _), _) = run(|s| (s.victim.inside, s.queue_empty) = (true, false));
    assert_eq!(code, Some(4));
    let ((code, posted), a) = run(|s| s.armed = false);
    assert_eq!(
        (code, a.state, posted),
        (Some(1), DONE, vec![(0x1fb, [600, 7, 0])])
    );
    let ((code, posted), a) = run(|_| {});
    assert_eq!((code, a.state, posted), (None, MOVE, vec![]));
}

/// State 6's step: ready in reach swings; ready on a dead tree ends with 5; a target out of
/// reach is followed (`set_state(4)`).
#[test]
fn a_ready_attack_swings_ends_on_a_dead_tree_or_follows() {
    let (code, a) = on_target(
        (2, 6, 2, [11.0, 10.0, 0.0]),
        true,
        (READY, false),
        |a, w, s| {
            a.arms.sprite = Some(STANDING);
            a.ready_step(w, s, true).unwrap()
        },
    );
    assert_eq!((code, a.state), (0, SWING));
    let (code, a) = on_target(
        (0, TREE, 3, [11.0, 10.0, 0.0]),
        true,
        (READY, false),
        |a, w, s| a.ready_step(w, s, false).unwrap(),
    );
    assert_eq!((code, a.state), (5, DONE));
    let (code, a) = on_target(
        (0, 6, 2, [15.0, 10.0, 0.0]),
        true,
        (READY, false),
        |a, w, s| {
            a.point = [NONE; 3];
            a.ready_step(w, s, true).unwrap()
        },
    );
    assert_eq!((code, a.state), (0, MOVE));
}

/// State 2's step after the kill: an animating swing sprite not yet looped keeps it; done, a
/// unit's own Attack lets its target go, posts 0x1fb with 600 and 0x202, ends and returns 2.
#[test]
fn a_killing_attack_ends_once_its_swing_looped() {
    let dead = (0, 6, 3, [11.0, 10.0, 0.0]);
    let ((code, let_go, posted), a) = on_target(dead, true, (KILLED, false), |a, w, s| {
        (a.killed_step(w, s).unwrap(), s.let_go, s.posted.clone())
    });
    assert_eq!(
        (code, let_go, posted, a.target, a.state),
        (2, Some(7), vec![(0x1fb, [600, 0, 0x202])], -1, DONE)
    );
    let (code, a) = on_target(dead, true, (KILLED, false), |a, w, s| {
        *s.sprite = Some(ActiveSprite::start(STANDING, &mut Well512::default()));
        a.killed_step(w, s).unwrap()
    });
    assert_eq!((code, a.state, a.target), (0, KILLED, 7));
}
