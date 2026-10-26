//! BUILD's tests before its foundation: a farm on rice terrain, the pending debit, the
//! builders, the placement test and the payment.

use crate::GenieError;
use crate::world::{FARM, RICE_FARM, World};

impl World {
    /// BUILD's first step (`FUN_140e7a400`): a farm (master 50) whose point's tile (truncated) is
    /// mangrove shallows or forest (terrains 54, 55, `FUN_1413fc100`) or a rice farm's (63 to 67,
    /// `FUN_1413fc220`) is a rice farm (1187); a point off the map keeps the farm.
    pub(crate) fn farm_on_rice_terrain(&self, master: i16, position: [f32; 3]) -> i16 {
        if master != FARM {
            return master;
        }
        let (x, y) = (position[0] as i32, position[1] as i32);
        let terrain = self.spatial.terrain.as_ref().and_then(|t| t.kind(x, y));
        match terrain {
            Some(54 | 55 | 63..=67) => RICE_FARM,
            _ => master,
        }
    }

    /// BUILD's pending debit (`FUN_141630d10` with 0, before its placement test): every cost
    /// entry of the master, paid or not, by resource, comes off the player's pending debit.
    pub(crate) fn debit_listed(&mut self, player: usize, master: i16) -> Result<(), GenieError> {
        let site =
            self.masters
                .get(player)
                .and_then(|m| m.get(master))
                .ok_or(GenieError::unsupported(
                    "BUILD of a master the player does not have",
                ))?;
        let mut listed = [0i16; 4];
        for &(resource, amount, _) in &site.cost {
            if let Some(r) = usize::try_from(resource).ok().filter(|&r| r < 4) {
                listed[r] = listed[r].wrapping_add(amount);
            }
        }
        for (debit, amount) in self.players[player].pending_debit.iter_mut().zip(listed) {
            *debit -= f32::from(amount);
        }
        Ok(())
    }

    /// BUILD's builders' test (`FUN_140e7a400` after the pending debit): a human player's BUILD
    /// needs one unit of the packet that can build the master ([`World::builds`]; one the world
    /// holds, `FUN_140e6bad0`, follows from it), else it places nothing; a computer's passes.
    pub(crate) fn build_has_builder(
        &self,
        player: usize,
        master: i16,
        builders: &[i32],
    ) -> Result<bool, GenieError> {
        if self.players[player].computer {
            return Ok(true);
        }
        let trained_at = self.master_of(player, master)?.trained_at.clone();
        for &id in builders {
            if self.builds((master, &trained_at), id)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether BUILD (`FUN_140e7a400`) places a foundation of `master` at `at` for `player`: its
    /// placement test ([`crate::production::spawn::build_placement`], the master's slot 0x50), then the
    /// payment's (`FUN_14162fb60` -> `FUN_141618db0`: every cost entry of a resource, paid or
    /// not, at most what the player holds). A refused BUILD places, pays and draws nothing (the
    /// player's slot 0x380 shows why). A cost in population (attribute 4) is not modelled.
    pub(crate) fn build_allowed(
        &self,
        player: usize,
        master: i16,
        at: [f32; 2],
    ) -> Result<bool, GenieError> {
        let human = !self.players[player].computer;
        if self.placement_code(player, master, at, human)? != 0 {
            return Ok(false);
        }
        self.affords(player, master)
    }

    /// The master's placement test (slot 0x50, [`crate::production::spawn::build_placement`]) of a
    /// foundation of `master` for `player` at `at`, `human` its explored test (`p6`): 0 when it
    /// may go there, else the game's code.
    pub(crate) fn placement_code(
        &self,
        player: usize,
        master: i16,
        at: [f32; 2],
        human: bool,
    ) -> Result<u8, GenieError> {
        self.placement_code_beside(player, master, at, human, &[])
    }

    /// [`World::placement_code`] whose scan passes over the objects `own` (the test's `p12` and
    /// its annexes).
    pub(crate) fn placement_code_beside(
        &self,
        player: usize,
        master: i16,
        at: [f32; 2],
        human: bool,
        own: &[i32],
    ) -> Result<u8, GenieError> {
        let site = self.master_of(player, master)?;
        if self.spatial.sight.is_none() {
            return Err(GenieError::unsupported(
                "BUILD's placement test without the players' sight",
            ));
        }
        let restriction = self
            .spatial
            .terrain
            .as_ref()
            .and_then(|t| {
                usize::try_from(site.terrain_restriction)
                    .ok()
                    .and_then(|r| t.restrictions.get(r))
            })
            .ok_or(GenieError::unsupported(
                "a foundation's terrain restriction the data file does not have",
            ))?;
        let build = crate::production::spawn::BuildSite {
            player,
            human,
            clearance: site.clearance,
            class: site.class,
            obstructs: !matches!(site.obstruction_type, 0 | 12),
            restriction,
            hill_mode: site.hill_mode,
            placement: site.placement,
            zones: false,
            own,
        };
        let map = crate::production::spawn::WorldMap { turns: self };
        crate::production::spawn::build_placement(&build, &map, at)
    }

    /// The payment's test of a foundation of `master` for `player` (`FUN_14162fb60` ->
    /// `FUN_141618db0`): every cost entry of a resource, paid or not, at most what the player
    /// holds. A cost in population (attribute 4) is not modelled.
    pub(crate) fn affords(&self, player: usize, master: i16) -> Result<bool, GenieError> {
        let site = self.master_of(player, master)?;
        let owner = &self.players[player];
        for &(resource, amount, _) in &site.cost {
            let Ok(r) = usize::try_from(resource) else {
                continue;
            };
            if r == 4 {
                return Err(GenieError::unsupported(
                    "a BUILD whose cost lists the population",
                ));
            }
            let held = if r < 4 {
                owner.resources[r]
            } else {
                owner.attributes.get(r).copied().unwrap_or(0.0)
            };
            if held < f32::from(amount) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
