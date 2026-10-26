//! The counts the update reads: the town centres, the trainable and the upgradable masters,
//! and where a master trains.

use crate::players::world_player::{
    AGES, BUILDABLE_TOWN_CENTRE, BUILDER, BUILDING_CLASS, RESEARCHED, TOWN_CENTRES, TreeQuery,
    attribute, full_tree_civ, get,
};
use crate::turn::Turns;

impl Turns {
    /// `FUN_141658970`: player `p`'s first built town centre (state 2) among the ids its tracker
    /// files under masters 109 and 444 (`FUN_14104f560`, `FUN_14104f4b0`: the first list, then
    /// the second), the objects the world holds.
    #[must_use]
    pub fn first_town_centre(&self, p: usize) -> Option<i32> {
        let tracker = &self.players[p].tracker;
        TOWN_CENTRES
            .iter()
            .flat_map(|&k| tracker.list(k).iter().copied())
            .find(|&id| self.object(id).is_some_and(|(_, u)| u.life.state == 2))
    }

    /// `FUN_1416597e0(player, n)`: the `n`-th object the world holds (`FUN_1415417a0`, in any
    /// state) among the ids player `p`'s tracker files under masters 109 and 444 (`FUN_14104f560`,
    /// `FUN_14104f4b0`: the first list, then the second).
    #[must_use]
    pub fn town_centre_at(&self, p: usize, n: usize) -> Option<i32> {
        let tracker = &self.players[p].tracker;
        TOWN_CENTRES
            .iter()
            .flat_map(|&k| tracker.list(k).iter().copied())
            .filter(|&id| self.object(id).is_some())
            .nth(n)
    }

    /// `FUN_141658a30`: player `p`'s town centres not dying (state below 3): those its tracker
    /// files under masters 109 and 444, then those waiting under 109's pending list
    /// (`FUN_14104fa50`, key + 90000).
    #[must_use]
    pub fn town_centre_count(&self, p: usize) -> i32 {
        let tracker = &self.players[p].tracker;
        let pending = tracker.list(TOWN_CENTRES[0] + crate::players::tracker::PENDING);
        TOWN_CENTRES
            .iter()
            .flat_map(|&k| tracker.list(k).iter())
            .chain(pending)
            .filter(|&&id| self.object(id).is_some_and(|(_, u)| u.life.state < 3))
            .count() as i32
    }

    /// `FUN_141625a20`'s count with no object (its fifth argument 0): the masters of player `p`,
    /// by id, of type 70 and up that the player has (`+0x5f0` not 0), that train at `building`
    /// ([`Turns::trains_at`]) and whose `+0x5f1` has no bit 0; for each, the limited units'
    /// test of its linked master (`FUN_140c7e1f0`) with `+0x5f1`'s bit 3 (`FUN_1416116c0`,
    /// `FUN_141611520`). With `building` 118 (a builder), attribute 48 at 0 and the town centre
    /// 621 neither had nor its `+0x5f1` set, one more when the town centres
    /// ([`Turns::town_centre_count`]) are fewer than attribute 218. genie's masters keep `+0x5f0`
    /// and `+0x5f1` as booleans ([`crate::masters::Master::available`],
    /// [`crate::masters::Master::disabled`]), a set one as 1: the limit bits 1 to 3 are none, and
    /// the limited units' tests not reached.
    #[must_use]
    pub fn trainable_count(&self, p: usize, building: i16) -> i16 {
        let Some(masters) = self.masters.get(p) else {
            return 0;
        };
        let mut count = 0i16;
        for m in masters.iter() {
            let flags = u8::from(m.disabled);
            if 70 <= m.kind && m.available && self.trains_at(m, building) && flags & 1 == 0 {
                count = count.wrapping_add(1);
            }
        }
        if building == BUILDER {
            let a = &self.players[p].attributes;
            let centre = masters.get(BUILDABLE_TOWN_CENTRE);
            if get(a, attribute::NO_TOWN_CENTRE) == 0.0
                && centre.is_some_and(|c| !c.disabled && !c.available)
                && (self.town_centre_count(p) as f32) < get(a, attribute::TOWN_CENTRE_LIMIT)
            {
                count = count.wrapping_add(1);
            }
        }
        count
    }

    /// `FUN_141610470`'s count with its fifth argument 1: the masters of player `p`, by id, of
    /// type 70 and up that train at `building` ([`Turns::trains_at`]): one with `+0x5f1`'s bit
    /// 1 or 2 counts; any other counts when the tech tree ([`TechTree`]) of the player's
    /// civilisation (the full tech tree's, `FUN_140c7e6f0`, under it) has it at the building
    /// within the ages (unless it is the civilisation's unique unit under the full tech tree:
    /// its elite one once the elite technology is researched, state 3), its link
    /// ([`TechTree::link`]) is no master of the player's or one it has, and its technology
    /// ([`TechTree::tech`]) is none of the player's or researched. With `building` 118,
    /// attribute 48 at 0 and the town centre 621 had with no `+0x5f1`, one more when the town
    /// centres are fewer than attribute 218. genie's masters keep `+0x5f0` and `+0x5f1` as
    /// booleans ([`Turns::trainable_count`]).
    ///
    /// [`TechTree::link`]: crate::players::world_player::tech_tree::TechTree::link
    /// [`TechTree::tech`]: crate::players::world_player::tech_tree::TechTree::tech
    /// [`TechTree`]: crate::players::world_player::tech_tree::TechTree
    #[must_use]
    pub fn upgradable_count(&self, p: usize, building: i16) -> i16 {
        let Some(masters) = self.masters.get(p) else {
            return 0;
        };
        let player = &self.players[p];
        let full = self.settings.full_tech_tree;
        let fallback = full_tree_civ(self.mode_680);
        let civ = if full {
            fallback
        } else {
            i32::from(player.civ)
        };
        let state = |t: i32| {
            usize::try_from(t)
                .ok()
                .and_then(|t| player.techs.get(t))
                .map(|t| t.state)
        };
        let ages_out = AGES.map(|t| state(t) == Some(-1));
        let unique = self
            .tech_tree
            .unique_units
            .get(usize::from(player.civ))
            .copied()
            .flatten()
            .map_or(-1, |u| match state(u.elite_tech) {
                Some(RESEARCHED) => u.elite,
                Some(_) => u.unit,
                None => -1,
            });
        let mut count = 0i16;
        for m in masters.iter() {
            if m.kind < 70 || !self.trains_at(m, building) {
                continue;
            }
            let flags = u8::from(m.disabled);
            if flags & 6 != 0 {
                count = count.wrapping_add(1);
                continue;
            }
            let q = TreeQuery {
                civ,
                building: i32::from(building),
                unit: i32::from(m.id),
                ages_out,
                settings: &self.settings,
                fallback,
            };
            if (!full || i32::from(m.id) != unique) && !self.tech_tree.has(&q) {
                continue;
            }
            let link = self.tech_tree.link(&q);
            let had = i16::try_from(link)
                .ok()
                .and_then(|l| masters.get(l))
                .map(|l| l.available);
            if had.is_none_or(|a| a) {
                let tech = self.tech_tree.tech(&q);
                if state(tech).is_none_or(|s| s == RESEARCHED) {
                    count = count.wrapping_add(1);
                }
            }
        }
        if building == BUILDER {
            let a = &player.attributes;
            let centre = masters.get(BUILDABLE_TOWN_CENTRE);
            if get(a, attribute::NO_TOWN_CENTRE) == 0.0
                && centre.is_some_and(|c| !c.disabled && c.available)
                && (self.town_centre_count(p) as f32) < get(a, attribute::TOWN_CENTRE_LIMIT)
            {
                count = count.wrapping_add(1);
            }
        }
        count
    }

    /// `FUN_141263610`: master `m` trains at `building`: one of its train locations (`+0x898`)
    /// is `building`, or the building's stack unit (`+0x986`) when the local player's master of
    /// it ([`Turns::local_player`], `FUN_140c88440`) is of class 3, disappears when built
    /// (`+0x984`) and has one (0 or more). A master without train locations trains nowhere.
    #[must_use]
    pub fn trains_at(&self, m: &crate::masters::Master, building: i16) -> bool {
        if m.trained_at.is_empty() {
            return false;
        }
        let mut at = building;
        let local = self
            .masters
            .get(self.local_player)
            .and_then(|t| t.get(building));
        if let Some(b) = local.filter(|b| b.class == BUILDING_CLASS as i16)
            && let Some(site) = b.building.as_ref()
            && site.disappears_when_built
            && 0 <= site.stack_unit
        {
            at = site.stack_unit;
        }
        m.trained_at.contains(&at)
    }
}
