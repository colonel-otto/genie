//! The income: the trickles, the mills' food, the attribute adds and their sharing, and the
//! update's tail.

use crate::math::fmath::at_most_or_unordered;
use crate::players::world_player::{
    BUILDING_TYPE, COMPUTER_TYPE, EARLY_SCOUTING_MS, GAIA_TYPE, LIVESTOCK_SCALE, MILL, MINUTE,
    SECOND, attribute, get, set,
};
use crate::turn::Turns;

impl Turns {
    /// The trickles of player `p`: each of attributes 230 to 233 above 0 (`COMISS` with 0,
    /// `JBE` skips a NaN) adds its sixtieth to food, wood, stone and gold in turn, through the
    /// attribute add with its statistic and its sharing ([`Turns::add_resource`] with 1 and 1).
    pub fn trickles(&mut self, p: usize) {
        for kind in 0..4 {
            let rate = get(&self.players[p].attributes, attribute::TRICKLES + kind);
            if at_most_or_unordered(rate, 0.0) {
                continue;
            }
            self.add_resource(p, kind, rate / MINUTE, 1, true);
        }
    }

    /// The food of player `p`'s mills' livestock: over the ids its tracker files under the mill
    /// (master 68, [`crate::players::tracker::Tracker::list`]), each object the world holds whose type is
    /// a building's sums its livestock (`mLivestockSummation`, `+0xc28`, in the list's order from
    /// 0); food grows by attribute 254 over 60 times (`time` times logf(sum / 200 + 1))
    /// ([`crate::math::fmath::log`]), with its statistic and its sharing ([`Turns::add_resource`]),
    /// also when that is 0. `time` is the update time before the loop's step.
    pub fn mill_food(&mut self, p: usize, time: f32) {
        let mut sum = 0.0;
        for &id in self.players[p].tracker.list(MILL) {
            let Some((owner, unit)) = self.object(id) else {
                continue;
            };
            let building = self
                .masters
                .get(owner)
                .and_then(|m| m.get(unit.life.master_id))
                .is_some_and(|m| m.kind == BUILDING_TYPE);
            if building {
                sum += unit.livestock_summation;
            }
        }
        let grows = crate::math::fmath::log(sum / LIVESTOCK_SCALE + SECOND);
        let rate = get(&self.players[p].attributes, attribute::MILL_FOOD) / MINUTE;
        self.add_resource(p, attribute::FOOD, rate * (time * grows), 1, true);
    }

    /// The world player's attribute add (slot 0xf8, `FUN_141616820`) of `amount` to resource
    /// `kind` (food, wood, stone, gold: 0 to 3) of player `p`, with `stat` (its fourth argument)
    /// and `share` (its fifth): with `stat` 1 the gathered total grows first (166 to 168; gold:
    /// any `stat` but 0 adds to attribute 49 unchecked, then 1 adds to 169), then the resource
    /// ([`Turns::add_attribute_base`]). The resource panel's event when a resource crosses 100
    /// (`FUN_140c898a0` 0x80) is the interface's. The kinds 50, 183 and 196 and the others are
    /// not resources: their branches are [`crate::turn::Player::add_attribute`]'s.
    pub fn add_resource(&mut self, p: usize, kind: usize, amount: f32, stat: u8, share: bool) {
        if kind == attribute::GOLD {
            if stat != 0 {
                let a = &mut self.players[p].attributes;
                let counter = get(a, attribute::GOLD_COUNTER);
                set(a, attribute::GOLD_COUNTER, amount + counter);
                if stat == 1 {
                    self.add_attribute_base(p, attribute::GOLD_GATHERED, amount, 1, share);
                }
            }
        } else if stat == 1 {
            self.add_attribute_base(p, attribute::GATHERED + kind, amount, 1, share);
        }
        self.add_attribute_base(p, kind, amount, stat, share);
    }

    /// The base player's attribute add (`FUN_14165d040`) of `amount` to attribute `kind` of
    /// player `p`, for a kind out of 0 to the attribute count nothing: the attribute takes
    /// `amount` plus its value (the resources are [`crate::turn::Player::resources`]). A
    /// computer player's resource fills its escrow: an `amount` above 0 with a share above 0
    /// (`COMISS`, `JBE`) adds the share times it; an escrow above the resource (`JBE` skips a
    /// NaN) falls to it, or to 0 when the resource is below 0. With `share`, a player who shares
    /// its gathered resources (`+0x1426`, [`crate::turn::Player::sharing`]) gives each resource
    /// to the players, in order, it counts as allies and who count it as theirs
    /// ([`Turns::shares_with`]): their slot 0xf8 with `stat` and no sharing. Attribute 11's peak
    /// (`+0x6fc`, unsaved) is not held; no resource add reaches it.
    pub(super) fn add_attribute_base(
        &mut self,
        p: usize,
        kind: usize,
        amount: f32,
        stat: u8,
        share: bool,
    ) {
        let player = &mut self.players[p];
        if player.attributes.len() <= kind {
            return;
        }
        let value = if kind < 4 {
            &mut player.resources[kind]
        } else {
            &mut player.attributes[kind]
        };
        *value += amount;
        if player.kind == COMPUTER_TYPE && kind < 4 {
            let pct = player.escrow_percentage[kind];
            if !at_most_or_unordered(amount, 0.0) && !at_most_or_unordered(pct, 0.0) {
                player.escrow[kind] += pct * amount;
            }
            let held = player.resources[kind];
            if !at_most_or_unordered(player.escrow[kind], held) {
                player.escrow[kind] = if at_most_or_unordered(0.0, held) {
                    held
                } else {
                    0.0
                };
            }
        }
        if share && player.sharing && kind < 4 {
            for q in 0..self.players.len() {
                if q != p && self.shares_with(p, q) {
                    self.add_resource(q, kind, amount, stat, false);
                }
            }
        }
    }

    /// `FUN_14165d040`'s test of the players a sharing player `p` gives to: player `q` is not
    /// Gaia and one of the world's, `p` counts it as an ally (relation 0 or 4), and `q` counts
    /// `p` as one (its relation to `p` 0 or 4, `& 0xfb`). A relation 1 would count both ways
    /// while the world's treaty holds (`FUN_1415e8e10`: `world+0x820`'s `+0x10` is 1), which the
    /// slice does not model.
    #[must_use]
    pub fn shares_with(&self, p: usize, q: usize) -> bool {
        let n = self.players.len();
        if q == 0 || n <= q || p == q {
            return false;
        }
        let mine = self.players[p].relations.get(q).copied();
        if !matches!(mine, Some(0 | 4)) {
            return false;
        }
        p != 0
            && p < n
            && self.players[q]
                .relations
                .get(p)
                .is_some_and(|&r| r & 0xfb == 0)
    }

    /// `WorldPlayer::update` after `WorldPlayerBase::update`: player `p`'s scout map (`+0x1370`,
    /// once made) takes 1 on each tile its tile list (`+0x180`) holds, in order: until the world
    /// time is 5000 ms only on the tiles the player sees ([`crate::spatial::Spatial::scout_sight`]
    /// is 0xf), after it on all. The list's new count (`+8`, `FUN_141662720`), which it reads and
    /// clears, is the list's length: only the AI's `FUN_140b90b90`, which a playback does not
    /// run, reads it too, and the player's update ends with the list emptied (`FUN_141661030`).
    /// Gaia's update has no tail.
    pub fn world_player_tail(&mut self, p: usize) {
        if self.players[p].kind == GAIA_TYPE || self.spatial.scout_map(p).is_none() {
            return;
        }
        let Some(tiles) = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.new_tiles.get(p))
            .cloned()
        else {
            return;
        };
        for [x, y] in tiles {
            let seen = self.spatial.scout_sight(p, x, y) == Some(0xf);
            if (self.world_time < EARLY_SCOUTING_MS && seen) || EARLY_SCOUTING_MS <= self.world_time
            {
                self.spatial.scout(p, x, y);
            }
        }
    }
}
