//! The sight of a combat object's update and its registry, an ownership change and a
//! construction's start and completion.

use crate::GenieError;
use crate::vision::registry::UnitUpdate;
use crate::vision::visibility::Visibility;
use crate::world::player::Player;

/// An object update past the registry check finds the registry gone.
const NO_REGISTRY: GenieError =
    GenieError::unreachable("an object update without the objects-in-sight registry");

/// `RGE_Combat_Object::update`'s takeover by sight (`0x14107dfb0`, once the registry took a
/// changed `mask` of an object with `+0x924` 1, [`Visibility::combat_sight`]): the players from
/// 1 on, in order, each with its computer flag (`player+0x10 == 3`, [`Player::computer`]) and
/// its `mAllowCapturingGaiaUnits` (`+0x670`, `allows`, the world's
/// `allow_gaia_capture`), as they stand. Outside the game modes whose descriptor's `+0x9c` is 1
/// (`standard`, `FUN_141302450`) a computer player that does not allow it is passed over; the
/// test `DAT_143e76fc3` guards before (`FUN_140ca6be0` on the AI's table) is never run, the
/// flag being 0 in the image and never written. The first other player than `owner` whose bit
/// is in `mask` ends the loop: it is the answer, which takes the object when it has its master.
#[must_use]
pub fn sight_taker(
    mask: u64,
    owner: usize,
    standard: bool,
    players: &[Player],
    allows: &[bool],
) -> Option<usize> {
    players
        .iter()
        .enumerate()
        .skip(1)
        .filter(|&(q, player)| {
            standard || !player.computer || allows.get(q).copied().unwrap_or(false)
        })
        .map(|(q, _)| q)
        .find(|&q| q != owner && mask & (1u64 << (q & 63)) != 0)
}

impl Visibility {
    /// `RGE_Combat_Object::update`'s sight (`0x14107dfb0`, after the actions): the object's mask
    /// is the unified map's word at its tile with those of its UMV entry's tiles on the map; an
    /// annex not yet dead (state below 7) reads its main building's (`main`: its place and UMV
    /// entry), with no test of the map's edge. A changed mask updates the registry with the place
    /// read, the master's radii and class (`Update_Unit_Info`), and is the answer for an object
    /// another player takes over by seeing it (`+0x924` 1, [`sight_taker`]).
    ///
    /// # Panics
    ///
    /// Never: the registry is checked before it is updated.
    pub fn combat_sight(
        &mut self,
        id: i32,
        owner: usize,
        at: [f32; 3],
        radius: [f32; 2],
        class: i16,
        main: Option<([f32; 3], u8)>,
    ) -> Result<Option<u64>, GenieError> {
        let Some(s) = self.seers.get(&id).copied() else {
            return Ok(None);
        };
        if self.registry.is_none() || id < 0 {
            return Ok(None);
        }
        let annex = main.filter(|_| s.state < 7 && s.building.is_some());
        let ([x, y, _], umv, edge) = match annex {
            Some((place, umv)) => (place, umv, false),
            None => (at, s.umv, true),
        };
        let mut mask = self.word_at(x, y)?;
        if umv != 0 {
            let offsets = self
                .extents
                .get(usize::from(umv) - 1)
                .map(|e| e.offsets.clone())
                .ok_or(GenieError::unsupported(
                    "an object's UMV entry beyond the list",
                ))?;
            for [dx, dy] in offsets {
                let (fx, fy) = (dx as f32 + x, dy as f32 + y);
                let on_map =
                    0.0 <= fx && 0.0 <= fy && fx < self.width as f32 && fy < self.height as f32;
                if on_map || !edge {
                    mask |= self.word_at(fx, fy)?;
                }
            }
        }
        let registry = self.registry.as_mut().ok_or(NO_REGISTRY)?;
        let old = registry.entry(id).map_or(0, |e| e.mask);
        if mask == old {
            return Ok(None);
        }
        registry.entries.entry(id).or_default().mask = mask;
        registry.update_unit(UnitUpdate {
            id,
            owner: owner as u8,
            position: [x, y],
            radii: radius,
            class,
            old: old & 0xffff_ffff,
            new: mask,
        })?;
        Ok((s.capture == 1).then_some(mask))
    }

    /// The registry's part of a combat object's new place (`FUN_14108c320`): its records follow.
    pub fn registry_moved(&mut self, id: i32, class: i16, from: [f32; 3], to: [f32; 3]) {
        if id < 0 || (from[0] == to[0] && from[1] == to[1]) {
            return;
        }
        if let Some(r) = self.registry.as_mut() {
            r.moved(id, class, [to[0], to[1]]);
        }
    }

    /// A master change's registry part (`FUN_14108c990`): the object leaves every list of its
    /// old class and joins those of its new one, with its mask as it was.
    pub fn registry_master(
        &mut self,
        id: i32,
        owner: usize,
        at: [f32; 3],
        old: ([f32; 2], i16),
        new: ([f32; 2], i16),
    ) -> Result<(), GenieError> {
        let Some(r) = self.registry.as_mut().filter(|_| 0 <= id) else {
            return Ok(());
        };
        let mask = r.entry(id).map_or(0, |e| e.mask);
        let place = [at[0], at[1]];
        r.update_unit(UnitUpdate {
            id,
            owner: owner as u8,
            position: place,
            radii: old.0,
            class: old.1,
            old: mask & 0xffff_ffff,
            new: 0,
        })?;
        r.update_unit(UnitUpdate {
            id,
            owner: owner as u8,
            position: place,
            radii: new.0,
            class: new.1,
            old: 0,
            new: mask,
        })
    }

    /// The combat object's destructor (`FUN_141073060`): it leaves every list.
    pub fn registry_leave(
        &mut self,
        id: i32,
        owner: usize,
        at: [f32; 3],
        radius: [f32; 2],
        class: i16,
    ) -> Result<(), GenieError> {
        let Some(r) = self.registry.as_mut().filter(|_| 0 <= id) else {
            return Ok(());
        };
        let mask = r.entry(id).map_or(0, |e| e.mask);
        r.update_unit(UnitUpdate {
            id,
            owner: owner as u8,
            position: [at[0], at[1]],
            radii: radius,
            class,
            old: mask & 0xffff_ffff,
            new: 0,
        })?;
        let _ = r.entries.remove(&id);
        Ok(())
    }

    /// An ownership change's records (`FUN_14120f800`, first of all and whatever the new owner's
    /// masters): an object with an id (`+0x8` not below 0) writes the new owner's id into each of
    /// its records (`+0x8b0`..`+0x8f0`), which keep their place in every list.
    pub fn records_owner(&mut self, id: i32, new: usize) -> Result<(), GenieError> {
        match self.registry.as_mut() {
            Some(r) if 0 <= id => r.owner_changed(id, new),
            _ => Ok(()),
        }
    }

    /// An ownership change's sight (`FUN_14120f800`, once the new owner has the master): an
    /// object that `sees` (state 2 and not inside another) loses its sight on the old owner's map
    /// (slot 0x1d8 with the old owner) and gains it on the new one's (slot 0x1d0) with `los`, the
    /// line of sight of the master of its own the change gave it (a livestock's takes the new
    /// owner's).
    pub fn owner_changed(
        &mut self,
        id: i32,
        position: [f32; 3],
        radius: [f32; 2],
        (new, los): (usize, f32),
        sees: bool,
    ) -> Result<(), GenieError> {
        if !self.seers.contains_key(&id) {
            return Ok(());
        }
        if sees {
            self.sight_off(id, position, radius, false, false)?;
        }
        if let Some(s) = self.seers.get_mut(&id) {
            s.player = new;
            s.los = los;
        }
        if sees {
            let _ = self.sight_on(id, position, radius, false, false)?;
        }
        Ok(())
    }

    /// A building's construction completes (`FUN_1411dbb70` with 2): once built, the sight of its
    /// footprint goes (slot 0x1d8, square) and that of its line of sight comes (slot 0x1d0), each
    /// on a tile corner when its radii add up to an even number.
    pub fn construction_completed(
        &mut self,
        id: i32,
        position: [f32; 3],
        radius: [f32; 2],
    ) -> Result<(), GenieError> {
        let Some(b) = self.seers.get_mut(&id).and_then(|s| s.building.as_mut()) else {
            return Ok(());
        };
        if b.built {
            return Ok(());
        }
        b.built = true;
        self.sight_off(id, position, radius, true, false)?;
        let _ = self.sight_on(id, position, radius, false, false)?;
        Ok(())
    }

    /// A foundation's construction starts (`FUN_1411e5670`): its build points become a sliver,
    /// its footprint comes into sight and its tiles link again.
    pub fn construction_started(
        &mut self,
        id: i32,
        position: [f32; 3],
        radius: [f32; 2],
    ) -> Result<(), GenieError> {
        if let Some(b) = self.seers.get_mut(&id).and_then(|s| s.building.as_mut()) {
            b.started = true;
        }
        let _ = self.sight_on(id, position, radius, true, false)?;
        let _ = self.link_building(position, radius)?;
        Ok(())
    }
}
