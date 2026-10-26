//! The damage beyond what a blast has done (`FUN_141082e80`) and the blast itself (slot
//! 0x848).

use crate::checkpoint::cvttss2si;
use crate::damage::{
    ARTIFACT, BUILDING, CONE, ELEPHANT_ARCHER, Kill, LIVESTOCK, MINUS_CONE, RAMS, Victim,
    WIDE_CONE, armor_damage, bonus_damage, charge_hit, elevation_factor, facing_offset, kill_bonus,
    lose, shield, take,
};
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
    let (from, to) = blast_tiles(center, r);
    let on_tiles = |t: &Victim| {
        let tile = [t.position[0] as i32, t.position[1] as i32];
        from[0] <= tile[0] && tile[0] < to[0] && from[1] <= tile[1] && tile[1] < to[1]
    };
    let spares_friends = !(1.0 < a.weapon_range && a.blast_level & 4 == 0);
    targets.sort_by_key(|t| (t.position[1] as i32, t.position[0] as i32, t.tile_place));
    // every building on the tiles starts the blast with nothing taken (+0xb90, first pass)
    for t in targets
        .iter_mut()
        .filter(|t| t.kind == BUILDING && on_tiles(t))
    {
        if let Some(b) = t.building.as_mut() {
            b.blast_taken = 0.0;
        }
    }
    attacker.source.charge_hit = false;
    let weapons = attacker.weapons.clone();
    for i in 0..targets.len() {
        let t = &*targets[i];
        if !on_tiles(t) {
            continue;
        }
        if t.uncertain {
            return Err(GenieError::unsupported(
                "a blast over an object whose player's update is not modelled",
            )
            .with("blasted", t.id));
        }
        if (spares_friends && t.allied[0])
            || Some(t.id) == exclude
            || t.id == attacker.id
            || t.class == ARTIFACT
        {
            continue;
        }
        let is_main = Some(t.id) == main;
        // a building keeps the blast's damage so far on its main building (+0xbb8), else on
        // itself; an annex reads its main building's area damage and blast level
        let ledger = if t.kind == BUILDING {
            match t.building.as_ref().and_then(|b| b.main) {
                Some(m) => Some(
                    targets
                        .iter()
                        .position(|v| v.id == m)
                        .ok_or(crate::build::NOT_HELD)?,
                ),
                None => Some(i),
            }
        } else {
            None
        };
        let taken = |v: &Victim| v.building.as_ref().map_or(0.0, |b| b.blast_taken);
        let (share, defense, blast_taken) = match ledger {
            Some(k) if k != i => {
                let m = &*targets[k];
                (m.source.area_damage, m.blast_defense, taken(m))
            }
            Some(_) => (a.area_damage, t.blast_defense, taken(t)),
            None => (a.area_damage, t.blast_defense, 0.0),
        };
        if !(1 < defense || a.blast_level <= defense) {
            continue;
        }
        let t = &mut *targets[i];
        if RAMS.contains(&a.master_id) && t.kind != BUILDING && !is_main {
            continue;
        }
        let gap = |d: f32, radius: f32| {
            if fmath::at_most_or_unordered(d, radius) {
                0.0
            } else {
                d - radius
            }
        };
        // the hit's slack (its info's +0xc, +0x10) is 0 for a melee hit
        let dx = gap((t.position[0] - center[0]).abs(), t.radius[0]);
        let dy = gap((t.position[1] - center[1]).abs(), t.radius[1]);
        let d2 = dy * dy + dx * dx;
        if fmath::at_most_or_unordered(r * r, d2) {
            continue;
        }
        if 70 <= t.kind && shield(t, false) {
            continue;
        }
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
        if !is_main {
            if a.blast_level & 0x80 != 0 {
                let facing = attacker.armored.as_ref().and_then(|x| x.facing).ok_or(
                    GenieError::unsupported("the facing of an attacker with a cone blast (+0x7e8)"),
                )?;
                if a.blast_level & 0x20 == 0 {
                    let off = facing_offset(attacker.position, facing, t.position);
                    if CONE < off || off < MINUS_CONE {
                        continue;
                    }
                } else {
                    if wide * wide < d2 {
                        continue;
                    }
                    let off = facing_offset(attacker.position, facing, t.position).abs();
                    if WIDE_CONE < off {
                        continue;
                    }
                }
            }
            // COMISS share, 0 / JC, then COMISS 0, share / JBE: a share below 0 is a fixed
            // damage, a NaN one goes on to the hit with the factor as it is
            if share < 0.0 {
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
                            to_gaia: false,
                        });
                        kill_bonus(&a)?;
                    }
                }
                continue;
            }
            if !share.is_nan() {
                f *= share;
            }
        }
        // the target leaves the slice while its hit may reach the others (its main building)
        let before = t.hp;
        let mut t = std::mem::replace(&mut *targets[i], Victim::placeholder());
        let hit = take(&mut t, attacker, targets, &weapons, f * factor, blast_taken);
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
            // ADDSS of the loss and the blast so far (commutative)
            if let Some(b) = ledger {
                b.blast_taken += lost;
            }
        }
        *targets[i] = t;
        hit?;
    }
    Ok(true)
}
