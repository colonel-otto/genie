use super::*;
use crate::unit_ai::best_target::choice::{add_extra, kept_by_owner_and_class, may_attack};

fn candidate(id: i32, distance: f32, speed: f32) -> Candidate {
    Candidate {
        id,
        distance,
        category: 1,
        class: 6,
        speed,
        armed: true,
        reach: 1.0,
        damage: 4.0,
        hp: 40.0,
        reload: 2.0,
        workable: false,
        current: false,
    }
}

/// `FUN_140c66590`: the bounds, the counts, the closest moving object, the kill time (hit
/// points over damage over reload: 40 / 4 / 2 = 5), 5 or the largest float without damage,
/// the fixed classes and the current target's bonus.
#[test]
fn the_statistics_count_bound_and_time_each_object() {
    let mut st = Statistics::default();
    st.add(&candidate(1, 3.0, 1.0));
    st.add(&Candidate {
        current: true,
        class: 3,
        damage: 0.0,
        workable: true,
        ..candidate(2, 0.5, 0.0)
    });
    st.add(&Candidate {
        damage: 0.0,
        category: -1,
        armed: false,
        ..candidate(3, 7.0, 2.0)
    });
    assert_eq!(st.distance, [0.5, 7.0]);
    assert_eq!(st.kill_time, [5.0, f32::MAX]);
    assert_eq!(st.moving_distance, [3.0, 7.0]);
    assert_eq!(st.moving_kill_time, [5.0, f32::MAX]);
    assert_eq!((st.others, st.moving, st.armed, st.in_reach), (2, 2, 2, 1));
    assert_eq!(st.by_category[1], 2);
    assert_eq!(st.closest_moving, (3.0, 1));
    let kill: Vec<f32> = st.entries.iter().map(|e| e.kill_time).collect();
    assert_eq!(kill, [5.0, 5.0, f32::MAX]);
    let fixed: Vec<bool> = st.entries.iter().map(|e| e.fixed).collect();
    assert_eq!(fixed, [false, true, false]);
    assert_eq!(st.entries[1].bonus, 5.0);
}

/// `FUN_140c48150`, then the rank and the sort: a still entry alone in its kind takes the
/// whole kill score (10) and, its distance span below the epsilon, 75; moving entries share
/// their spans; the weight is its category's times 10. The sort ranks the highest first,
/// a tie to the lower id.
#[test]
fn the_scores_rank_and_sort_the_entries() {
    let mut st = Statistics::default();
    st.add(&candidate(5, 2.0, 1.0));
    st.add(&Candidate {
        hp: 80.0,
        ..candidate(4, 6.0, 1.0)
    });
    st.add(&candidate(9, 4.0, 0.0));
    let mut weights = [0.0; 12];
    weights[1] = 0.5;
    st.score(&weights, (0, 0), &mut |_| Ok(false)).unwrap();
    let scores: Vec<(f32, f32, f32, f32)> = st
        .entries
        .iter()
        .map(|e| (e.kill_score, e.distance_score, e.weight, e.total))
        .collect();
    // moving kill times 5 and 10: 10 - (0/5)·10 = 10 and 10 - (5/5)·10 = 0; moving
    // distances 2 and 6: 75 and 0; the still entry: kill span 10 - 5 over all, distance
    // span 6 - 2 over all
    assert_eq!(
        scores,
        [
            (10.0, 75.0, 5.0, 90.0),
            (0.0, 0.0, 5.0, 5.0),
            (10.0, 37.5, 5.0, 52.5)
        ]
    );
    st.rank(&mut |e| Ok(i32::from(!e.moving))).unwrap();
    st.sort().unwrap();
    let ids: Vec<i32> = st.order.iter().map(|&i| st.entries[i].id).collect();
    assert_eq!(ids, [9, 5, 4]);
    // equal ranks go to the lower id
    for e in &mut st.entries {
        e.rank = 1.0;
    }
    st.order = vec![0, 1, 2];
    st.sort().unwrap();
    let ids: Vec<i32> = st.order.iter().map(|&i| st.entries[i].id).collect();
    assert_eq!(ids, [4, 5, 9]);
}

/// The order history's weights: the younger an order, the more its category weighs; the
/// first one older than a minute ends the walk.
#[test]
fn the_order_history_weighs_the_last_minute_s_categories() {
    let record = |time, category| crate::unit_ai::OrderRecord {
        time,
        category,
        ..crate::unit_ai::OrderRecord::default()
    };
    let w = order_weights(
        &[
            record(100_000, 1),
            record(70_000, 1),
            record(30_000, 3),
            record(99_000, 2),
        ],
        100_000,
    );
    assert_eq!(w[1], 1.0 + 30_000.0 / 60_000.0);
    assert_eq!(w[3], 0.0);
    assert_eq!(w[2], 0.0);
}

/// A world of objects by id: each a [`Thing`] and its candidate's values; the paths that
/// succeed and the calls made; the object checks it refuses (`'a'` the attack check, `'f'`
/// the friends' scan, `'n'` the neutral test) and the ones made, in order.
#[derive(Default)]
struct Field {
    things: std::collections::BTreeMap<i32, (Thing, Candidate)>,
    quick: Vec<i32>,
    paths: Vec<i32>,
    asked: Vec<(char, i32)>,
    refused: Vec<char>,
    checked: Vec<char>,
}

impl Field {
    /// Check `c` is made: it passes unless refused.
    fn check(&mut self, c: char) -> bool {
        self.checked.push(c);
        !self.refused.contains(&c)
    }
}

impl Battlefield for Field {
    fn thing(&mut self, id: i32) -> Result<Option<Thing>, crate::GenieError> {
        Ok(self.things.get(&id).map(|t| t.0))
    }
    fn can_attack(&mut self, _: i32) -> Result<bool, crate::GenieError> {
        Ok(self.check('a'))
    }
    fn attacks_neutral(&mut self, _: i16) -> Result<bool, crate::GenieError> {
        Ok(self.check('n'))
    }
    fn spares_friends(&mut self, _: i32) -> Result<bool, crate::GenieError> {
        Ok(self.check('f'))
    }
    fn shuns(&mut self, _: i32) -> Result<bool, crate::GenieError> {
        Ok(false)
    }
    fn candidate(
        &mut self,
        id: i32,
        distance: f32,
        category: i32,
    ) -> Result<Candidate, crate::GenieError> {
        Ok(Candidate {
            distance,
            category,
            ..self.things[&id].1
        })
    }
    fn armored(&mut self, _: i32) -> Result<bool, crate::GenieError> {
        Ok(false)
    }
    fn weapon_range(&mut self, _: i32) -> Result<f32, crate::GenieError> {
        Ok(0.0)
    }
    fn engages(&mut self, _: i32) -> Result<bool, crate::GenieError> {
        Ok(true)
    }
    fn quick_path(&mut self, id: i32, _: f32) -> Result<bool, crate::GenieError> {
        self.asked.push(('q', id));
        Ok(self.quick.contains(&id))
    }
    fn path(&mut self, id: i32, _: f32) -> Result<bool, crate::GenieError> {
        self.asked.push(('p', id));
        Ok(self.paths.contains(&id))
    }
}

fn soldier() -> Searcher {
    Searcher {
        id: 1,
        target: -1,
        target_types: [-1, -1],
        order: None,
        action: None,
        master: 74,
        class: 6,
        kind: 70,
        attack_priority: 1,
        can_move: true,
        sight: 12.0,
        range: 0.5,
        weapon_range: 0.5,
        speed: 1.0,
        now: 10_000,
        ..Searcher::default()
    }
}

fn enemy(id: i32, class: i16, distance: f32, hp: f32) -> (Thing, Candidate) {
    (
        Thing {
            owner: 2,
            state: 2,
            has_ai: true,
            master: 4,
            class,
            kind: 70,
            targetable: true,
            attackable: true,
            distance,
            speed: 1.0,
            ..Thing::default()
        },
        Candidate {
            speed: 1.0,
            hp,
            ..candidate(id, distance, 1.0)
        },
    )
}

/// `bestUnitToAttack` over three enemies (an idle check, path checks on): the statistics
/// rank them (the nearest's distance score outweighs the weakest's kill score: 7, 9, 8); the
/// quick paths of the first three (moving objects) are asked, 9's answers.
#[test]
fn the_best_unit_to_attack_ranks_then_takes_the_first_quick_path() {
    let mut field = Field::default();
    let _ = field.things.insert(7, enemy(7, 0, 3.0, 40.0));
    let _ = field.things.insert(8, enemy(8, 0, 9.0, 40.0));
    let _ = field.things.insert(9, enemy(9, 0, 5.0, 10.0));
    field.quick = vec![9];
    let lists = [vec![], vec![], vec![], vec![], vec![8, 7, 9]];
    let ask = Ask {
        idle: true,
        paths: 1,
        any_class: true,
        ..Ask::default()
    };
    let c = best_unit_to_attack(&soldier(), &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, 9);
    assert_eq!(field.asked, [('q', 7), ('q', 9)]);
    assert_eq!(c.retries, []);
    // without a quick path the paths follow; 7's fails (a moving object: ignored for 1 s)
    field.quick.clear();
    field.paths = vec![9];
    field.asked.clear();
    let c = best_unit_to_attack(&soldier(), &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, 9);
    assert_eq!(
        field.asked,
        [('q', 7), ('q', 9), ('q', 8), ('p', 7), ('p', 9)]
    );
    assert_eq!(c.retries, [(7, 11_000)]);
}

/// Each of the first three whose path fails is ignored for 1 s (a moving object) or 2 s
/// (a still one), and nothing found answers −1. A retarget entry's target waits for the
/// retry walk, which runs only without other statistics.
#[test]
fn failed_paths_become_retarget_entries() {
    let mut field = Field::default();
    let _ = field.things.insert(7, enemy(7, 0, 3.0, 40.0));
    let mut still = enemy(9, 0, 5.0, 10.0);
    still.0.speed = 0.0;
    still.1.speed = 0.0;
    let _ = field.things.insert(9, still);
    let lists = [vec![], vec![], vec![], vec![], vec![7, 9]];
    let ask = Ask {
        idle: true,
        paths: 1,
        any_class: true,
        ..Ask::default()
    };
    let c = best_unit_to_attack(&soldier(), &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, -1);
    let mut retries = c.retries;
    retries.sort_unstable();
    assert_eq!(retries, [(7, 11_000), (9, 12_000)]);
    let me = Searcher {
        retries: vec![9],
        ..soldier()
    };
    field.asked.clear();
    let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, -1);
    assert_eq!(field.asked, [('q', 7), ('p', 7)]);
    // with 7 a retarget entry too, the retry walk takes both
    let me = Searcher {
        retries: vec![9, 7],
        ..soldier()
    };
    field.paths = vec![9];
    field.asked.clear();
    let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, 9);
}

/// The lists' records pass the objects without a unit AI, inside another, dead or allied,
/// and those beyond the sight (twice the line of sight: 12) or under the least range; the
/// one left answers.
#[test]
fn the_lists_pass_over_what_the_unit_may_not_attack() {
    let mut field = Field::default();
    let mut no_ai = enemy(30, 0, 3.0, 40.0);
    no_ai.0.has_ai = false;
    let mut inside = enemy(31, 0, 3.0, 40.0);
    inside.0.inside = true;
    let mut dead = enemy(32, 0, 3.0, 40.0);
    dead.0.state = 3;
    let mut ally = enemy(33, 0, 3.0, 40.0);
    ally.0.allied = true;
    for (id, t) in [(30, no_ai), (31, inside), (32, dead), (33, ally)] {
        let _ = field.things.insert(id, t);
    }
    let _ = field.things.insert(34, enemy(34, 0, 12.5, 40.0));
    let _ = field.things.insert(35, enemy(35, 0, 0.1, 40.0));
    let _ = field.things.insert(36, enemy(36, 0, 11.9, 40.0));
    field.quick = (30..=36).collect();
    let lists = [vec![], vec![], vec![], vec![], (30..=36).collect()];
    let ask = Ask {
        idle: true,
        paths: 1,
        any_class: true,
        ..Ask::default()
    };
    let me = Searcher {
        min_range: 0.2,
        ..soldier()
    };
    let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, 36);
    assert_eq!(field.asked, [('q', 36)]);
}

/// A unit that attacks walls (its last target a wall) and finds only walls and gates takes
/// the closest gate; one that does not passes them over.
#[test]
fn walls_and_gates_count_for_a_unit_that_attacks_walls() {
    let mut field = Field::default();
    let _ = field.things.insert(20, enemy(20, 27, 2.0, 100.0));
    let _ = field.things.insert(21, enemy(21, 39, 4.0, 100.0));
    let _ = field.things.insert(22, enemy(22, 39, 3.0, 100.0));
    let lists = [vec![], vec![], vec![], vec![], vec![20, 21, 22]];
    let ask = Ask {
        idle: true,
        paths: 1,
        any_class: true,
        ..Ask::default()
    };
    let me = Searcher {
        target_types: [-1, 27],
        ..soldier()
    };
    let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, 22);
    let c = best_unit_to_attack(&soldier(), &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, -1);
}

/// `FUN_140c4c8e0`'s switch: the archers' class 0 is 3, but masters 6 and 7 (the crossbowman
/// lines' first) are 4; the siege's 13 is 6, but the rams' masters are 5; a class outside the
/// table is −1.
#[test]
fn the_categories_follow_the_classes_and_their_exceptions() {
    assert_eq!(category(4, 0), 3);
    assert_eq!(category(6, 0), 4);
    assert_eq!(category(7, 23), 4);
    assert_eq!(category(83, 4), 7);
    assert_eq!(category(74, 6), 1);
    assert_eq!(category(1744, 6), 5);
    assert_eq!(category(280, 13), 6);
    assert_eq!(category(35, 13), 5);
    assert_eq!(category(1258, 55), 5);
    assert_eq!(category(109, 3), 0);
    assert_eq!(category(448, 47), 1);
    assert_eq!(category(1, 1), -1);
    assert_eq!(category(1, 63), -1);
    assert_eq!(category(1, -1), -1);
}

/// `bestUnitToAttack`'s extra objects ([`add_extra`]): each the world holds but
/// the current target counts at its distance; one under the least range counts beyond the
/// weapon range (its gap plus the weapon range plus 1), and not at all for a unit that does
/// not move.
#[test]
fn an_extra_object_under_the_least_range_counts_beyond_the_weapon_range() {
    let mut field = Field::default();
    for (id, d) in [(40, 3.0), (41, 0.1), (42, 4.0)] {
        let _ = field.things.insert(id, enemy(id, 0, d, 40.0));
    }
    let ask = Ask {
        extra: vec![40, 41, 42, 43],
        ..Ask::default()
    };
    let me = Searcher {
        target: 42,
        min_range: 0.2,
        ..soldier()
    };
    let counted = |me: &Searcher, field: &mut Field| {
        let mut st = Statistics::default();
        add_extra(me, &ask, &mut st, field).unwrap();
        st.entries
            .iter()
            .map(|e| (e.id, e.distance))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        counted(&me, &mut field),
        [(40, 3.0), (41, 0.1_f32 + 0.5 + 1.0)]
    );
    let still = Searcher { speed: 0.0, ..me };
    assert_eq!(counted(&still, &mut field), [(40, 3.0)]);
}

/// A gate answers at once only when nothing but walls and gates counts: with an enemy among
/// them the choice goes on to the quick paths.
#[test]
fn a_gate_answers_at_once_only_among_walls() {
    let mut field = Field::default();
    let _ = field.things.insert(20, enemy(20, 27, 2.0, 100.0));
    let _ = field.things.insert(22, enemy(22, 39, 3.0, 100.0));
    let _ = field.things.insert(7, enemy(7, 0, 5.0, 40.0));
    field.quick = vec![7];
    let lists = [vec![], vec![], vec![], vec![], vec![20, 22, 7]];
    let ask = Ask {
        idle: true,
        paths: 1,
        any_class: true,
        ..Ask::default()
    };
    let me = Searcher {
        target_types: [-1, 27],
        ..soldier()
    };
    let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, 7);
    assert!(field.asked.contains(&('q', 7)));
}

/// Outside an idle check the current target answers by its path when it moves and engages
/// (without path checks at once); its path failing, the quick paths follow. An idle check
/// takes no such shortcut.
#[test]
fn the_current_target_answers_by_its_path_outside_an_idle_check() {
    let mut field = Field::default();
    let _ = field.things.insert(7, enemy(7, 0, 3.0, 40.0));
    let _ = field.things.insert(9, enemy(9, 0, 5.0, 40.0));
    field.paths = vec![7];
    let lists = [vec![], vec![], vec![], vec![], vec![7, 9]];
    let ask = Ask {
        paths: 1,
        any_class: true,
        ..Ask::default()
    };
    let me = Searcher {
        target: 7,
        ..soldier()
    };
    let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
    assert_eq!((c.target, &field.asked[..]), (7, &[('p', 7)][..]));
    field.asked.clear();
    let at_once = Ask {
        paths: 0,
        ..ask.clone()
    };
    let c = best_unit_to_attack(&me, &at_once, &lists, &mut field).unwrap();
    assert_eq!((c.target, field.asked.len()), (7, 0));
    field.paths.clear();
    field.quick = vec![9];
    let c = best_unit_to_attack(&me, &ask, &lists, &mut field).unwrap();
    assert_eq!(c.target, 9);
    assert_eq!(field.asked[0], ('p', 7));
    field.asked.clear();
    let idle = Ask { idle: true, ..ask };
    field.paths = vec![7];
    let c = best_unit_to_attack(&me, &idle, &lists, &mut field).unwrap();
    assert_eq!(c.target, 9);
    assert!(!field.asked.contains(&('p', 7)));
}

/// The world's checks of a candidate ([`may_attack`]), in the game's order: one without a unit
/// AI, dead, allied or inside another fails before any; then the attack check, a combat unit's
/// friends' scan (a unit of another type skips it) and the neutral test of list 3 alone, each
/// failing it.
#[test]
fn a_candidate_passes_the_attack_check_the_friends_scan_and_the_neutral_test_in_order() {
    let (thing, _) = enemy(7, 0, 3.0, 40.0);
    let checked = |me: &Searcher, t: &Thing, kind, refused: &[char]| {
        let mut field = Field {
            refused: refused.to_vec(),
            ..Field::default()
        };
        let pass = may_attack(me, (7, t), kind, &mut field).unwrap();
        (pass, field.checked)
    };
    let me = soldier();
    assert_eq!(checked(&me, &thing, 4, &[]), (true, vec!['a', 'f']));
    assert_eq!(checked(&me, &thing, 3, &[]), (true, vec!['a', 'f', 'n']));
    assert_eq!(checked(&me, &thing, 3, &['a']), (false, vec!['a']));
    assert_eq!(checked(&me, &thing, 3, &['f']), (false, vec!['a', 'f']));
    assert_eq!(
        checked(&me, &thing, 3, &['n']),
        (false, vec!['a', 'f', 'n'])
    );
    let building = Searcher {
        kind: 80,
        ..soldier()
    };
    assert_eq!(checked(&building, &thing, 4, &[]), (true, vec!['a']));
    for t in [
        Thing {
            has_ai: false,
            ..thing
        },
        Thing { state: 3, ..thing },
        Thing {
            allied: true,
            ..thing
        },
        Thing {
            inside: true,
            ..thing
        },
    ] {
        assert_eq!(checked(&me, &t, 4, &[]), (false, vec![]));
    }
}

/// The owner and class rules ([`kept_by_owner_and_class`]): without `any_class` a building is
/// passed over; a computer player's unit passes over Gaia's objects that hold something or that
/// others take by sight, and boats unless it is a boat itself, of class 52 or a boat hunter (the
/// castle); a human player's unit keeps them all.
#[test]
fn a_computer_s_unit_passes_over_gaia_s_holdings_and_boats() {
    let (thing, _) = enemy(7, 0, 3.0, 40.0);
    let any = Ask {
        any_class: true,
        ..Ask::default()
    };
    let human = soldier();
    let computer = Searcher {
        computer: true,
        ..soldier()
    };
    let house = Thing { class: 3, ..thing };
    assert!(!kept_by_owner_and_class(&human, &Ask::default(), &house));
    assert!(kept_by_owner_and_class(&human, &any, &house));
    let gaia = Thing { owner: 0, ..thing };
    let holding = Thing { held: 5.0, ..gaia };
    let taken = Thing {
        capture: true,
        ..gaia
    };
    assert!(kept_by_owner_and_class(&computer, &any, &gaia));
    // a held amount that is not a number counts as held (`+0x310 <= 0` fails)
    let unordered = Thing {
        held: f32::NAN,
        ..gaia
    };
    for t in [holding, taken, unordered] {
        assert!(!kept_by_owner_and_class(&computer, &any, &t));
        assert!(kept_by_owner_and_class(&human, &any, &t));
    }
    let boat = Thing { class: 22, ..thing };
    assert!(!kept_by_owner_and_class(&computer, &any, &boat));
    assert!(kept_by_owner_and_class(&human, &any, &boat));
    for me in [
        Searcher {
            class: 52,
            ..computer.clone()
        },
        Searcher {
            master: 82,
            ..computer.clone()
        },
        Searcher {
            class: 2,
            ..computer
        },
    ] {
        assert!(kept_by_owner_and_class(&me, &any, &boat));
    }
}
