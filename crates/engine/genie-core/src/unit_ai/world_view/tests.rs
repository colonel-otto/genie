use super::*;

/// Slot 0x228 (`FUN_141084070`): the attacks above 0 add up in their order from +0; none
/// (or none above 0) is +0.
#[test]
fn the_attacks_sum_is_the_positive_ones() {
    let m = |weapons: Vec<(i16, i16)>| crate::objects::masters::Master {
        weapons,
        ..crate::objects::masters::Master::default()
    };
    assert_eq!(
        attacks(&m(vec![(4, 6), (3, -2), (11, 1)])).to_bits(),
        7.0f32.to_bits()
    );
    assert_eq!(attacks(&m(vec![])).to_bits(), 0.0f32.to_bits());
    assert_eq!(
        attacks(&m(vec![(4, 0), (3, -5)])).to_bits(),
        0.0f32.to_bits()
    );
}
/// The villager 123 (player 1) tests player 2's unit 50 of `master`: harmless when its
/// master has object flag 0x10, is its own copy and shows an attack below 5 (4 only below 60
/// hit points), no melee armor and a range of at most 0.5.
fn harmless(master: crate::objects::masters::Master) -> bool {
    let me = Unit {
        id: 123,
        life: crate::objects::lifecycle::Lifecycle {
            master_id: 83,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    let enemy = Player {
        objects: vec![Unit {
            id: 50,
            life: crate::objects::lifecycle::Lifecycle {
                master_id: master.id,
                state: 2,
                ..crate::objects::lifecycle::Lifecycle::default()
            },
            ..Unit::default()
        }],
        ..Player::default()
    };
    let (gaia, after) = (Player::default(), [enemy]);
    let before = [gaia];
    let every = [
        Masters::default(),
        Masters::default(),
        Masters::new([master]),
    ];
    let spatial = Spatial::new([120, 120]);
    let attacker = crate::unit_ai::Attacker {
        ai_type: 8,
        player: 1,
        computer: false,
        state: 2,
        held: -1,
        top_action: None,
        master: 83,
        kind: 70,
        class: 4,
        attack_priority: 0,
        missile: -1,
        range: 0.0,
        min_range: 0.0,
        charge_type: 0,
        area_effect: 0.0,
        blast_level: 0,
        los: 4.0,
        position: [10.0, 10.0, 0.0],
        radius: [0.2, 0.2],
        speed: 0.8,
        scenario: false,
        firepower: 0.0,
        capture: 0,
    };
    let picker = WorldPicker {
        attacker: &attacker,
        seeker: None,
        me: 123,
        player: 1,
        others: &[],
        foreign: Foreign {
            before: &before,
            after: &after,
            left: None,
        },
        spatial: &spatial,
        every: &every,
        restriction: -1,
        unit: &me,
        attributes: &[],
        relations: &[0, 0, 0],
        ai_settings: crate::world::player::AiSettings::default(),
        now: 0,
        mode_680: 0,
    };
    picker.harmless(50).unwrap()
}

/// Slot 0x78 of the ward's unit AI: of its attackers the world holds, the nearest by the
/// footprint gap, the first of a tie; −1 for none, None for a ward without a unit AI.
#[test]
fn a_ward_s_nearest_attacker_is_its_closest_held_one() {
    let master = |id, radius| crate::objects::masters::Master {
        id,
        kind: 70,
        radius,
        ..crate::objects::masters::Master::default()
    };
    let unit = |id, x, master_id, ai: Option<UnitAi>| Unit {
        id,
        position: [x, 10.0, 0.0],
        ai,
        life: crate::objects::lifecycle::Lifecycle {
            master_id,
            state: 2,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    let attacked = |attackers| UnitAi {
        attackers,
        ..UnitAi::default()
    };
    let me = unit(123, 10.0, 4, None);
    let others = [
        unit(20, 12.0, 4, Some(attacked(vec![50, 51, 52, 53]))),
        unit(21, 12.0, 4, None),
        unit(22, 12.0, 4, Some(attacked(vec![99]))),
    ];
    let enemy = Player {
        // 50 at a gap of 4.4, 51 of 2.4 (its radius 1), 53 of 2.4 too
        objects: vec![
            unit(50, 17.0, 5, None),
            unit(51, 16.0, 6, None),
            unit(53, 16.0, 6, None),
        ],
        ..Player::default()
    };
    let (gaia, after) = (Player::default(), [enemy]);
    let before = [gaia];
    let every = [
        Masters::default(),
        Masters::new([master(4, [0.3, 0.3])]),
        Masters::new([master(5, [0.3, 0.3]), master(6, [1.3, 0.3])]),
    ];
    let spatial = Spatial::new([120, 120]);
    let attacker = crate::unit_ai::Attacker {
        ai_type: 13,
        player: 1,
        computer: false,
        state: 2,
        held: -1,
        top_action: None,
        master: 4,
        kind: 70,
        class: 6,
        attack_priority: 0,
        missile: -1,
        range: 0.0,
        min_range: 0.0,
        charge_type: 0,
        area_effect: 0.0,
        blast_level: 0,
        los: 4.0,
        position: [10.0, 10.0, 0.0],
        radius: [0.3, 0.3],
        speed: 1.0,
        scenario: false,
        firepower: 0.0,
        capture: 0,
    };
    let picker = WorldPicker {
        attacker: &attacker,
        seeker: None,
        me: 123,
        player: 1,
        others: &others,
        foreign: Foreign {
            before: &before,
            after: &after,
            left: None,
        },
        spatial: &spatial,
        every: &every,
        restriction: -1,
        unit: &me,
        attributes: &[],
        relations: &[0, 0, 0],
        ai_settings: crate::world::player::AiSettings::default(),
        now: 0,
        mode_680: 0,
    };
    assert_eq!(picker.nearest_attacker_of(20), Some(51));
    assert_eq!(picker.nearest_attacker_of(21), None);
    assert_eq!(picker.nearest_attacker_of(22), Some(-1));
    assert_eq!(picker.nearest_attacker_of(77), None);
}

#[test]
fn a_villager_fights_back_a_weak_attacker_of_its_own_master() {
    let villager = crate::objects::masters::Master {
        id: 83,
        copy_id: 83,
        kind: 70,
        class: 4,
        object_flags: 0x10,
        hit_points: 25,
        weapons: vec![(4, 3), (21, 6)],
        armors: vec![(4, 0), (3, 0)],
        weapon_range: 0.0,
        ..crate::objects::masters::Master::default()
    };
    assert!(harmless(villager.clone()));
    let with = |f: &dyn Fn(&mut crate::objects::masters::Master)| {
        let mut m = villager.clone();
        f(&mut m);
        harmless(m)
    };
    assert!(!with(&|m| m.object_flags = 0));
    assert!(!with(&|m| m.copy_id = 293));
    assert!(!with(&|m| m.weapons = vec![(4, 5)]));
    // an attack of 4 below 60 hit points
    assert!(with(&|m| m.weapons = vec![(4, 4)]));
    assert!(!with(&|m| (m.weapons, m.hit_points) = (vec![(4, 4)], 60)));
    assert!(with(&|m| (m.weapons, m.hit_points) = (vec![(4, 3)], 60)));
    assert!(!with(&|m| m.armors = vec![(4, 1)]));
    assert!(with(&|m| m.weapon_range = 0.5));
    assert!(!with(&|m| m.weapon_range = 0.6));
    assert!(!with(&|m| m.weapon_range = f32::NAN));
}

/// What the statistics read of a candidate (`FUN_140c44680`'s reads): soldier 123 (master 4,
/// attack 6 of class 4, reload 2) of player 1 reads player 2's unit 50 (master 5, class 6,
/// attack 3, 40 hit points, no armor, standing): armed by its attacks, the soldier's range,
/// 6 damage a hit, its reload, the candidate's distance and category as given; not the
/// seeker's target, not a conversion. An object the world does not hold is not modelled.
#[test]
fn a_candidate_s_statistics_are_its_class_hit_points_and_the_damage_to_it() {
    use crate::unit_ai::AttackPicker;
    let master = |id, class, weapons| crate::objects::masters::Master {
        id,
        kind: 70,
        class,
        radius: [0.3, 0.3],
        weapons,
        reload: 2.0,
        // no terrain defense bonus: the damage reads no terrain
        terrain_bonus: -1,
        ..crate::objects::masters::Master::default()
    };
    let unit = |id, x, master_id| Unit {
        id,
        position: [x, 10.0, 0.0],
        life: crate::objects::lifecycle::Lifecycle {
            master_id,
            state: 2,
            hp: 40.0,
            ..crate::objects::lifecycle::Lifecycle::default()
        },
        ..Unit::default()
    };
    let me = unit(123, 10.0, 4);
    let enemy = Player {
        objects: vec![unit(50, 12.0, 5)],
        ..Player::default()
    };
    let (gaia, after) = (Player::default(), [enemy]);
    let before = [gaia];
    let every = [
        Masters::default(),
        Masters::new([master(4, 6, vec![(4, 6)])]),
        Masters::new([master(5, 6, vec![(4, 3)])]),
    ];
    let spatial = Spatial::new([120, 120]);
    let attacker = crate::unit_ai::Attacker {
        ai_type: 13,
        player: 1,
        computer: false,
        state: 2,
        held: -1,
        top_action: None,
        master: 4,
        kind: 70,
        class: 6,
        attack_priority: 0,
        missile: -1,
        range: 0.5,
        min_range: 0.0,
        charge_type: 0,
        area_effect: 0.0,
        blast_level: 0,
        los: 4.0,
        position: [10.0, 10.0, 0.0],
        radius: [0.3, 0.3],
        speed: 1.0,
        scenario: false,
        firepower: 0.0,
        capture: 0,
    };
    let mut picker = WorldPicker {
        attacker: &attacker,
        seeker: None,
        me: 123,
        player: 1,
        others: &[],
        foreign: Foreign {
            before: &before,
            after: &after,
            left: None,
        },
        spatial: &spatial,
        every: &every,
        restriction: -1,
        unit: &me,
        attributes: &[],
        relations: &[0, 0, 0],
        ai_settings: crate::world::player::AiSettings::default(),
        now: 0,
        mode_680: 0,
    };
    let c = picker.candidate(50, 1.4, 2).unwrap();
    assert_eq!(
        (c.id, c.distance, c.category, c.class, c.speed),
        (50, 1.4, 2, 6, 0.0)
    );
    assert_eq!(
        (c.armed, c.reach, c.damage, c.hp, c.reload),
        (true, 0.5, 6.0, 40.0, 2.0)
    );
    assert_eq!((c.workable, c.current), (false, false));
    assert!(picker.candidate(77, 1.4, 2).is_err());
}
