//! The damage beyond what a blast has done (`FUN_141082e80`) and the blast itself (slot
//! 0x848).

use crate::combat::damage::{
    ARTIFACT, Attacker, BUILDING, CONE, ELEPHANT_ARCHER, Kill, LIVESTOCK, MINUS_CONE, RAMS, Victim,
    WIDE_CONE, armor_damage, bonus_damage, charge_hit, elevation_factor, facing_offset, kill_bonus,
    lose, shield, take,
};
use crate::math::conversions::cvttss2si;
use crate::{GenieError, math::fmath};

/// `FUN_141082e80`: the damage ([`armor_damage`]) beyond what a blast has done: an attacker of
/// type 70 and up records whether the hit is its charge's (slot 0x2f0, `+0x8a8`), which empties
/// a charge without an area effect (slot 0x318 with 0); the target loses it (slot 0xb0). The
/// target's unit AI gets notify 500 (but from an attacker of class 64) with the attacker, the hit
/// points left and its master's, sent by the attacker (a missile, type 60: by what it was fired
/// from, `+0x998`). Below 1 hit point it has 0 and is killed: Gaia takes a player's livestock
/// (type 70). Not modelled: an attacker of type 60 and up whose combat ability marks its target
/// (bit 2, `FUN_141206250`) and a kill reward task (action type 0xa1). Not held: the damaged flag (`+0x683`), the last damage
/// time (`+0x648`), the alerts of another owner (slot 0x2e8), the killer (`+0x690`, `+0x694`),
/// the local player's statistics, the damage sprites (slot 0x4c0) and the trigger events.
pub(super) fn damage(
    t: &mut Victim,
    src: &mut Victim,
    weapons: &[(i16, i16)],
    factor: f32,
    blast_taken: f32,
) -> Result<(), GenieError> {
    let d = armor_damage(t, weapons, factor, &src.source, bonus_damage(src, t))?;
    if blast_taken < d {
        if 70 <= src.kind {
            let hit = charge_hit(&src.source, t);
            src.source.charge_hit = hit;
            if hit && src.source.area_effect <= 0.0 {
                src.source.charge = 0.0;
            }
        }
        lose(t, d - blast_taken);
    }
    let marks = t.kind == 70 || (t.kind == 80 && matches!(t.class, 51 | 54));
    if 60 <= src.kind && src.source.ability & 4 != 0 && marks {
        return Err(GenieError::unsupported(
            "an attacker whose combat ability marks its target (FUN_141206250)",
        ));
    }
    let master_hp = i32::from(t.master_hp);
    if src.source.class != ELEPHANT_ARCHER {
        let sender = if src.kind == 60 {
            src.source.fired_from
        } else {
            src.id
        };
        t.damaged.push((sender, [src.id, t.hp as i32, master_hp]));
    }
    if t.hp < 1.0 {
        t.hp = 0.0;
        if t.armored.as_ref().is_some_and(|a| a.kill_tasks) {
            return Err(GenieError::unsupported(
                "a kill reward task (action type 0xa1)",
            ));
        }
        t.killed = Some(Kill {
            attacker: src.id,
            attacker_player: src.player,
            attacker_class: src.class,
            to_gaia: t.kind == 70 && t.class == LIVESTOCK && t.player != 0,
        });
    }
    Ok(())
}

/// The blast's rectangle of tiles about `center` for radius `r` (`FUN_14107f690`): the tiles
/// from `(int)(c - r) - 6` (at least 0) to before `(int)(c + r) + 6` (at most the map's edge,
/// which no object passes), each `CVTTSS2SI` into 32 bits kept as a short.
pub(super) fn blast_tiles(center: [f32; 3], r: f32) -> ([i32; 2], [i32; 2]) {
    let low = |c: f32| {
        let t = (cvttss2si(c - r) as i16).wrapping_sub(6);
        i32::from(t.max(0))
    };
    let high = |c: f32| i32::from((cvttss2si(c + r) as i16).wrapping_add(6));
    (
        [low(center[0]), low(center[1])],
        [high(center[0]), high(center[1])],
    )
}

/// Slot 0x848 (`FUN_14107f690`), the blast of `attacker` about `center` with factor `factor`,
/// over the objects the tiles of [`blast_tiles`] hold, row by row and a tile's objects in its
/// list's order ([`Victim::tile_place`]): the attacker's hit is first marked as no
/// charge's (`+0x8a8`). An object is passed when the blast spares its friends (a weapon range of
/// at most 1 or the level's bit 2: `FUN_1410bfcb0`) and its player is the attacker's or allied,
/// when it is `exclude`, the attacker or an artifact, when its `area_effect_object_level` is
/// at most 1 and below the blast's level, when a ram's blast meets neither a building nor `main`,
/// when it lies beyond the radius (the gap past its radii) or its shield blocks the blast. Its
/// damage factor is the attacker's elevation factor, times its `friendly_fire_damage` when it is
/// of type 70 and up and its player counts the source's as its own or allied; times the falloff
/// `(r - d) / r` (a melee attacker's only with the level's bit 6, and not on `main`; a missile's
/// everywhere, at distance 0 on a `main` building). An object other than `main` lies within the
/// cone of a cone blast (bit 7: within half a facet, or 1.67 for a wide one, bit 5) and takes the
/// attacker's `area_damage` share; a fixed damage (`area_damage` below 0) takes its hit points
/// alone (slot 0xb0; below 1 it has 0 and holds nothing, `FUN_14119d320`; a combat unit of a
/// player that had 1 is counted killed, `FUN_1412005b0`). Then the object's slot 0xe8 with the
/// attacker's weapons ([`take`]), and the attacker's hit is its charge's if any hit was
/// (slot 0x2f0). Every building on the tiles starts the blast with nothing taken (`+0xb90`, a
/// first pass over the tiles); a building's hit leaves out what the blast has done to it so far,
/// its main building's for an annex (`+0xbb8`, which also gives the annex its `area_damage`
/// share and its blast level, the main building's master's), and adds its loss to it; the
/// main building of an annex it reaches is among `targets`. The compares are the assembly's: a NaN gap past a radius is 0
/// (`COMISS`/`JBE`), a NaN square distance is beyond the radius (`JBE`), a NaN facet offset
/// stays in a cone (`JA` past, both the narrow and the wide one), a NaN `area_damage` leaves the
/// factor as it is. Answers whether the radius is above 0 (or NaN: `COMISS 0, r`/`JC`), which
/// neither caller reads.
pub fn blast(
    attacker: &mut Victim,
    center: [f32; 3],
    exclude: Option<i32>,
    main: Option<i32>,
    targets: &mut [&mut Victim],
    factor: f32,
) -> Result<bool, GenieError> {
    let a = attacker.source;
    let r = a.area_effect;
    if !fmath::below_or_unordered(0.0, r) {
        return Ok(false);
    }
    let wide = if a.blast_level & 0x80 != 0 { r } else { 0.0 };
    let blast = Blast {
        a,
        center,
        r,
        wide,
        tiles: blast_tiles(center, r),
        spares_friends: !(1.0 < a.weapon_range && a.blast_level & 4 == 0),
        exclude,
        main,
        factor,
    };
    targets.sort_by_key(|t| (t.position[1] as i32, t.position[0] as i32, t.tile_place));
    // every building on the tiles starts the blast with nothing taken (+0xb90, first pass)
    for t in targets
        .iter_mut()
        .filter(|t| t.kind == BUILDING && blast.on_tiles(t))
    {
        if let Some(b) = t.building.as_mut() {
            b.blast_taken = 0.0;
        }
    }
    attacker.source.charge_hit = false;
    let weapons = attacker.weapons.clone();
    for i in 0..targets.len() {
        blast_one(attacker, targets, i, (&blast, &weapons))?;
    }
    Ok(true)
}

/// One blast as each object reads it ([`blast`]): the attacker's values at its start, its
/// center, radius and wide cone's radius (0 without one), its tiles ([`blast_tiles`]), whether
/// it spares the attacker's friends, the object it leaves out, its `main` object and its factor.
#[derive(Clone, Copy, Debug)]
pub(super) struct Blast {
    pub(super) a: Attacker,
    pub(super) center: [f32; 3],
    pub(super) r: f32,
    pub(super) wide: f32,
    pub(super) tiles: ([i32; 2], [i32; 2]),
    pub(super) spares_friends: bool,
    pub(super) exclude: Option<i32>,
    pub(super) main: Option<i32>,
    pub(super) factor: f32,
}

impl Blast {
    /// Whether the object stands on the blast's tiles (its place truncated).
    pub(super) const fn on_tiles(&self, t: &Victim) -> bool {
        let (from, to) = self.tiles;
        let tile = [t.position[0] as i32, t.position[1] as i32];
        from[0] <= tile[0] && tile[0] < to[0] && from[1] <= tile[1] && tile[1] < to[1]
    }
}

/// The blast on object `i` of `targets` with the attacker's `weapons` ([`blast`]): passed off
/// its tiles, as a friend it spares, the object it leaves out, the attacker, an artifact, an
/// object of a blast level it defends, a ram's non-building other than `main`, one beyond the
/// radius, one whose shield blocks it, and one outside a cone; a fixed damage takes its size
/// ([`fixed_blast`]); any other takes the hit ([`blast_hit`]).
pub(super) fn blast_one(
    attacker: &mut Victim,
    targets: &mut [&mut Victim],
    i: usize,
    (b, weapons): (&Blast, &[(i16, i16)]),
) -> Result<(), GenieError> {
    let a = &b.a;
    let t = &*targets[i];
    if !b.on_tiles(t) {
        return Ok(());
    }
    if t.uncertain {
        return Err(GenieError::unsupported(
            "a blast over an object whose player's update is not modelled",
        )
        .with("blasted", t.id));
    }
    if (b.spares_friends && t.allied[0])
        || Some(t.id) == b.exclude
        || t.id == attacker.id
        || t.class == ARTIFACT
    {
        return Ok(());
    }
    let is_main = Some(t.id) == b.main;
    let ledger = blast_ledger(targets, i)?;
    let (share, defense, blast_taken) = ledger_values(targets, i, ledger, a);
    if !(1 < defense || a.blast_level <= defense) {
        return Ok(());
    }
    let t = &mut *targets[i];
    if RAMS.contains(&a.master_id) && t.kind != BUILDING && !is_main {
        return Ok(());
    }
    let Some(d2) = blast_distance2(t, b.center, b.r) else {
        return Ok(());
    };
    if 70 <= t.kind && shield(t, false) {
        return Ok(());
    }
    let mut f = blast_falloff(attacker, a, t, (b.r, d2), is_main);
    if !is_main {
        if outside_cone(attacker, a, t, (d2, b.wide))? {
            return Ok(());
        }
        // COMISS share, 0 / JC, then COMISS 0, share / JBE: a share below 0 is a fixed
        // damage, a NaN one goes on to the hit with the factor as it is
        if share < 0.0 {
            return fixed_blast(attacker, a, t, share);
        }
        if !share.is_nan() {
            f *= share;
        }
    }
    blast_hit(
        attacker,
        targets,
        i,
        (weapons, f * b.factor),
        (ledger, blast_taken),
    )
}

/// The object a building's blast damage so far is kept on (`+0xbb8`): its main building's
/// place among `targets` for an annex, its own for another building; None for an object that
/// is not a building.
pub(super) fn blast_ledger(targets: &[&mut Victim], i: usize) -> Result<Option<usize>, GenieError> {
    let t = &*targets[i];
    if t.kind != BUILDING {
        return Ok(None);
    }
    t.building.as_ref().and_then(|b| b.main).map_or_else(
        || Ok(Some(i)),
        |m| {
            targets
                .iter()
                .position(|v| v.id == m)
                .map(Some)
                .ok_or(crate::construction::build::NOT_HELD)
        },
    )
}

/// What the blast reads of object `i` through its `ledger`: the `area_damage` share, the blast
/// level it defends to and the blast's damage so far; an annex reads its main building's.
pub(super) fn ledger_values(
    targets: &[&mut Victim],
    i: usize,
    ledger: Option<usize>,
    a: &Attacker,
) -> (f32, u8, f32) {
    let taken = |v: &Victim| v.building.as_ref().map_or(0.0, |b| b.blast_taken);
    match ledger {
        Some(k) if k != i => {
            let m = &*targets[k];
            (m.source.area_damage, m.blast_defense, taken(m))
        }
        Some(_) => (a.area_damage, targets[i].blast_defense, taken(&*targets[i])),
        None => (a.area_damage, targets[i].blast_defense, 0.0),
    }
}

/// The square distance from the blast's `center` to the object's box (the gap past each
/// radius, a NaN gap 0: `COMISS`/`JBE`); None beyond the radius `r` (a NaN square distance is
/// beyond it: `JBE`). The hit's slack (its info's `+0xc`, `+0x10`) is 0 for a melee hit.
pub(super) fn blast_distance2(t: &Victim, center: [f32; 3], r: f32) -> Option<f32> {
    let gap = |d: f32, radius: f32| {
        if fmath::at_most_or_unordered(d, radius) {
            0.0
        } else {
            d - radius
        }
    };
    let dx = gap((t.position[0] - center[0]).abs(), t.radius[0]);
    let dy = gap((t.position[1] - center[1]).abs(), t.radius[1]);
    let d2 = dy * dy + dx * dx;
    if fmath::at_most_or_unordered(r * r, d2) {
        None
    } else {
        Some(d2)
    }
}

/// The blast's damage factor on `t` at square distance `d2` within radius `r`: the attacker's
/// elevation factor, times `t`'s `friendly_fire_damage` when it is of type 70 and up and its
/// player is the attacker's or allied, times the falloff `(r - d) / r` (a melee attacker's only
/// with the level's bit 6 and not on `main`; a missile's everywhere, at distance 0 on a `main`
/// building).
pub(super) fn blast_falloff(
    attacker: &Victim,
    a: &Attacker,
    t: &Victim,
    (r, d2): (f32, f32),
    is_main: bool,
) -> f32 {
    let mut f = elevation_factor(attacker.position[2], attacker.attack_elevation, t);
    if 70 <= t.kind && (t.player == attacker.player || t.allied[1]) {
        f *= t.armored.as_ref().map_or(1.0, |x| x.friendly_fire);
    }
    if a.missile < 0 {
        if a.blast_level & 0x40 != 0 && !is_main {
            f *= (r - fmath::sqrt(d2)) / r;
        }
    } else {
        let d = if !is_main || t.kind != BUILDING {
            fmath::sqrt(d2)
        } else {
            0.0
        };
        f *= (r - d) / r;
    }
    f
}

/// Whether `t` lies outside the cone of a cone blast (the level's bit 7): beyond half a facet
/// of the attacker's facing, or for a wide one (bit 5) beyond the wide radius or 1.67 facets; a
/// NaN facet offset stays in either. A blast of no cone has none to leave.
pub(super) fn outside_cone(
    attacker: &Victim,
    a: &Attacker,
    t: &Victim,
    (d2, wide): (f32, f32),
) -> Result<bool, GenieError> {
    if a.blast_level & 0x80 == 0 {
        return Ok(false);
    }
    let facing =
        attacker
            .armored
            .as_ref()
            .and_then(|x| x.facing)
            .ok_or(GenieError::unsupported(
                "the facing of an attacker with a cone blast (+0x7e8)",
            ))?;
    if a.blast_level & 0x20 == 0 {
        let off = facing_offset(attacker.position, facing, t.position);
        return Ok(CONE < off || off < MINUS_CONE);
    }
    if wide * wide < d2 {
        return Ok(true);
    }
    let off = facing_offset(attacker.position, facing, t.position).abs();
    Ok(WIDE_CONE < off)
}

/// A fixed blast damage (an `area_damage` `share` below 0) on `t`: it loses the share's size
/// alone (slot 0xb0); below 1 it has 0 hit points and holds nothing (`FUN_14119d320`); a combat
/// unit of a player that had 1 is counted killed (`FUN_1412005b0`).
pub(super) fn fixed_blast(
    attacker: &Victim,
    a: &Attacker,
    t: &mut Victim,
    share: f32,
) -> Result<(), GenieError> {
    let before = t.hp;
    lose(t, share.abs());
    if t.hp < 1.0 {
        t.hp = 0.0;
        t.amount = 0.0;
        if f32::from(t.capacity) < t.amount {
            t.amount = f32::from(t.capacity);
        }
        if 1.0 <= before && 0 < t.player && t.kind == 70 {
            t.killed = Some(Kill {
                attacker: attacker.id,
                attacker_player: attacker.player,
                attacker_class: attacker.class,
                to_gaia: false,
            });
            kill_bonus(a)?;
        }
    }
    Ok(())
}

/// The blast's hit on object `i` with the attacker's `weapons` and `factor` ([`take`]): the
/// object leaves the slice while its hit may reach the others (its main building); a hit marks
/// the attacker's as its charge's, and its loss goes onto the `ledger` past `blast_taken`
/// (`ADDSS`, commutative).
pub(super) fn blast_hit(
    attacker: &mut Victim,
    targets: &mut [&mut Victim],
    i: usize,
    (weapons, factor): (&[(i16, i16)], f32),
    (ledger, blast_taken): (Option<usize>, f32),
) -> Result<(), GenieError> {
    let before = targets[i].hp;
    let mut t = std::mem::replace(&mut *targets[i], Victim::placeholder());
    let hit = take(&mut t, attacker, targets, weapons, factor, blast_taken);
    if hit.is_ok() {
        if !attacker.source.charge_hit {
            attacker.source.charge_hit = charge_hit(&attacker.source, &t);
        }
        let lost = before - t.hp;
        let ledger = match ledger {
            Some(k) if k == i => t.building.as_mut(),
            Some(k) => targets[k].building.as_mut(),
            None => None,
        };
        if let Some(b) = ledger {
            b.blast_taken += lost;
        }
    }
    *targets[i] = t;
    hit
}
