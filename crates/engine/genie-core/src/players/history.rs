//! The player's history (`TRIBE_History_Info`, TribeHistoryInfo.cpp; player `+0xdb8`, saved with
//! the player): a population entry each history period and a list of events, newest first.
//!
//! The world player's update (`WorldPlayer::update`, `0x14163b100`) runs [`History::update`] once
//! a game second; every `time_slice` runs (the `.dat`'s, `world+0x8b8`: 30) it adds an entry
//! (attributes 37 and 40, the civilian and military population) and follows the player's kills
//! and razings (attributes 20, 43, 152 to 155, 170 and 172 against its own value, 164 and 165)
//! for battles. The events: an age reached (research of 101 to 103, `FUN_1413cdcd0`), a castle
//! or wonder built (the building's `set_state`, slot 20, `FUN_1411dbb70`) or destroyed (on its
//! destroyer's history, slot 29, `FUN_1411cea90`), a battle ([`History::update`]) and a defeat
//! (the status setter, slot 1, `FUN_141639860`).
//!
//! The simulation reads it back: the AI's current age time (`FUN_140b077d0`,
//! [`History::age_time`]) is the time since the age's event. The post-game timeline reads the
//! entries and every event ([`History::events_at`], `FUN_1419f7b90`); the achievements and the
//! post-game statistics read the age events.
//!
//! 158041: `add_history_event` `FUN_141560ae0`, `add_new_history_entry` `FUN_141560b90`, the
//! first event of a kind `FUN_141560e20`, `get_history_event_by_time` `FUN_141560ea0`,
//! `mark_new_battle` `FUN_141565470`, `update` `FUN_141569450`; the names are the 2016 HD PDB's.

use crate::checkpoint::{cvttss2si, cvttss2si_64};
use crate::math::image::history::{BATTLE_RECORD, BATTLE_START};

/// Feudal Age reached (research 101).
pub const FEUDAL_AGE: u8 = 0;
/// Castle Age reached (research 102).
pub const CASTLE_AGE: u8 = 1;
/// Imperial Age reached (research 103).
pub const IMPERIAL_AGE: u8 = 2;
/// A wonder built (master 276); `data1` is its owner.
pub const WONDER_BUILT: u8 = 4;
/// A wonder destroyed, on its destroyer's history; `data1` is its owner.
pub const WONDER_DESTROYED: u8 = 5;
/// A castle built (master 82); `data1` is its owner.
pub const CASTLE_BUILT: u8 = 6;
/// A castle destroyed, on its destroyer's history; `data1` is its owner.
pub const CASTLE_DESTROYED: u8 = 7;
/// A battle: `data1` the opponent, `data2` its share of the player's value, `data3` 1.
pub const BATTLE: u8 = 11;
/// A defeat: `data1` the player with the most of attribute 300 + its own id, `data2` that value.
pub const DEFEAT: u8 = 20;

/// The entries' buffer grows by this many (`add_new_history_entry`).
const ENTRY_GROWTH: i32 = 60;
/// The battles a history keeps before a new one has to push one out (`mark_new_battle`).
const BATTLES_KEPT: usize = 3;
/// The newest battles `mark_new_battle` looks among for one to push out, the new one included.
const BATTLES_COMPARED: i32 = 3;
/// The first of the attributes that count the player's kills' value of each player (400 + i).
const KILL_VALUE_OF: usize = 400;
/// The first of the attributes that count the player's razings' value of each player (425 + i).
const RAZING_VALUE_OF: usize = 425;

/// The player's attributes the update reads.
mod attribute {
    pub const KILLS: usize = 20;
    pub const CIVILIAN_POPULATION: usize = 37;
    pub const MILITARY_POPULATION: usize = 40;
    pub const RAZINGS: usize = 43;
    pub const KILLS_VALUE_OTHER: usize = 152;
    pub const RAZINGS_VALUE_OTHER: usize = 153;
    pub const KILLS_OTHER: usize = 154;
    pub const RAZINGS_OTHER: usize = 155;
    pub const UNITS_VALUE: usize = 164;
    pub const BUILDINGS_VALUE: usize = 165;
    pub const KILLS_VALUE: usize = 170;
    pub const RAZINGS_VALUE: usize = 172;
}

/// An event (`Time_Slice_History_Event`, 0x20 bytes, `calloc`ed).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Event {
    /// `event` (`+0x0`): one of the kinds above.
    pub kind: u8,
    /// `time_slice` (`+0x4`): the number of entries when it happened.
    pub time_slice: i32,
    /// `world_time` (`+0x8`): the world's time then, in milliseconds.
    pub world_time: u32,
    /// `data1` (`+0xc`).
    pub data1: f32,
    /// `data2` (`+0x10`).
    pub data2: f32,
    /// `data3` (`+0x14`).
    pub data3: f32,
}

/// The player's history (`TRIBE_History_Info`), every field the save holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    /// `history` (`+0x0`) up to `number_of_entries` (`+0x1c`): each entry's civilian and
    /// military population.
    pub entries: Vec<[i16; 2]>,
    /// `max_number_of_entries` (`+0x20`): the entries' buffer, grown by 60.
    pub max_entries: i32,
    /// `events` (`+0x8`), newest first (the list from its head).
    pub events: Vec<Event>,
    /// `number_of_events` (`+0x18`): as loaded; no writer counts it.
    pub number_of_events: i32,
    /// `update_history_count` (`+0x28`): the update's runs since the last period.
    pub update_count: i32,
    /// `old_total_razings` (`+0x2c`): attribute 43 at the last period.
    pub old_total_razings: i32,
    /// `old_build_value_razings` (`+0x30`): attribute 172.
    pub old_build_value_razings: i32,
    /// `old_total_razings_other` (`+0x34`): attribute 155.
    pub old_total_razings_other: i32,
    /// `old_build_value_razings_other` (`+0x38`): attribute 153.
    pub old_build_value_razings_other: i32,
    /// `old_total_kills` (`+0x3c`): attribute 20.
    pub old_total_kills: i32,
    /// `old_build_value_kills` (`+0x40`): attribute 170.
    pub old_build_value_kills: i32,
    /// `old_total_kills_other` (`+0x44`): attribute 154.
    pub old_total_kills_other: i32,
    /// `old_build_value_kills_other` (`+0x48`): attribute 152.
    pub old_build_value_kills_other: i32,
    /// `razings_weight` (`+0x4c`): saved; no routine of 158041 reads it.
    pub razings_weight: f32,
    /// `kills_weight` (`+0x50`): saved; no routine reads it.
    pub kills_weight: f32,
    /// `razing_percent` (`+0x54`): saved; no routine reads it.
    pub razing_percent: f32,
    /// `kills_perecent` (`+0x58`): saved; no routine reads it.
    pub kills_percent: f32,
    /// `razings_flag` (`+0x5c`): saved; no routine reads it.
    pub razings_flag: u32,
    /// `battle_flag` (`+0x60`): a battle goes on.
    pub battle_flag: u32,
    /// `old_current_build_value_units` (`+0x64`): attribute 164.
    pub old_current_build_value_units: i32,
    /// `old_current_build_value_buildings` (`+0x68`): attribute 165.
    pub old_current_build_value_buildings: i32,
    /// `old_player_kill_BV` (`+0xb0`, `numMaxPlayers` of them): attribute 400 + i, outside a
    /// battle.
    pub old_player_kill_bv: Vec<i32>,
    /// `old_player_razing_BV` (`+0xc8`): attribute 425 + i, outside a battle.
    pub old_player_razing_bv: Vec<i32>,
    /// `running_ave_BV_percent` (`+0xe0`): the battle's share of the player's value so far.
    pub running_ave_bv_percent: f32,
    /// `running_total_BV_kills` (`+0xe4`).
    pub running_total_bv_kills: i32,
    /// `running_total_BV_razings` (`+0xe8`).
    pub running_total_bv_razings: i32,
    /// `running_total_kills` (`+0xec`; the save keeps its low 16 bits).
    pub running_total_kills: i32,
    /// `running_total_razings` (`+0xf0`; the save keeps its low 16 bits).
    pub running_total_razings: i32,
}

/// A float through `CVTTSS2SI` and a 16-bit `MOVSX` back to a float, as the update hands the
/// populations to `add_new_history_entry`.
fn short(x: f32) -> f32 {
    f32::from(cvttss2si(x) as i16)
}

impl History {
    /// `add_new_history_entry` (`FUN_141560b90`): the buffer grows by 60 once full (its old
    /// capacity copied, the rest zero); an entry of kind 0 holds the two values as shorts
    /// (`CVTTSS2SI`), any other kind leaves its slot zero; the count goes up either way.
    pub fn add_entry(&mut self, kind: u8, civilian: f32, military: f32) {
        if self.max_entries <= self.entries.len() as i32 {
            self.max_entries = self.max_entries.wrapping_add(ENTRY_GROWTH);
        }
        self.entries.push(if kind == 0 {
            [cvttss2si(civilian) as i16, cvttss2si(military) as i16]
        } else {
            [0, 0]
        });
    }

    /// `add_history_event` (`FUN_141560ae0`): with at least one entry, an event of `kind` at
    /// the entries' count and the world's time joins the list's head (`data3` the byte as a
    /// float); a battle then goes through [`History::mark_new_battle`].
    pub fn add_event(&mut self, kind: u8, data1: f32, data2: f32, data3: u8, world_time: u32) {
        if self.entries.is_empty() {
            return;
        }
        self.events.insert(
            0,
            Event {
                kind,
                time_slice: self.entries.len() as i32,
                world_time,
                data1,
                data2,
                data3: f32::from(data3),
            },
        );
        if kind == BATTLE {
            let _ = self.mark_new_battle();
        }
    }

    /// `mark_new_battle` (`FUN_141565470`) on the new battle at the list's head: its `data3`
    /// becomes 1. With more than three battles, the one among the three newest (the new one
    /// first) whose `data2` lies furthest below the new one's goes (`COMISS` with `JBE`: a NaN
    /// never qualifies, the first of equal gaps wins); with none below it the new battle goes
    /// instead. True unless the new battle went.
    pub fn mark_new_battle(&mut self) -> bool {
        let battles = self.events.iter().filter(|e| e.kind == BATTLE).count();
        let Some(new) = self.events.first_mut() else {
            return true;
        };
        new.data3 = 1.0;
        let new = new.data2;
        if battles <= BATTLES_KEPT {
            return true;
        }
        let mut seen = 0;
        let mut widest = 0.0f32;
        let mut furthest = None;
        for (i, e) in self.events.iter().enumerate() {
            if BATTLES_COMPARED <= seen {
                break;
            }
            if e.kind != BATTLE {
                continue;
            }
            seen += 1;
            if new > e.data2 {
                let gap = new - e.data2;
                if gap > widest {
                    widest = gap;
                    furthest = Some(i);
                }
            }
        }
        if let Some(i) = furthest {
            let _ = self.events.remove(i);
            true
        } else {
            let _ = self.events.remove(0);
            false
        }
    }

    /// `FUN_141560e20`: the newest event of `kind`.
    #[must_use]
    pub fn event_of_kind(&self, kind: u8) -> Option<&Event> {
        self.events.iter().find(|e| e.kind == kind)
    }

    /// `get_history_event_by_time` (`FUN_141560ea0`, the post-game timeline's): the events of
    /// `time_slice`, as the position of the first from the head (1 based) and their count (a
    /// byte, wrapping); none gives `None` (the game's -1 and -1).
    #[must_use]
    pub fn events_at(&self, time_slice: i32) -> Option<(i32, i8)> {
        let mut first = None;
        let mut count = 0i8;
        for (i, e) in self.events.iter().enumerate() {
            if e.time_slice == time_slice {
                count = count.wrapping_add(1);
                let _ = first.get_or_insert(i as i32 + 1);
            }
        }
        first.map(|f| (f, count))
    }

    /// The AI's current age time (`FUN_140b077d0`, its fact's): the world's seconds, less those
    /// of the event of the player's age (attribute 6 through `CVTTSS2SI`: 1 Feudal, 2 Castle, 3
    /// Imperial) when the history holds one; for another age the world's seconds.
    #[must_use]
    pub fn age_time(&self, current_age: f32, world_time: u32) -> u32 {
        let now = world_time / 1000;
        let kind = match cvttss2si(current_age) {
            1 => FEUDAL_AGE,
            2 => CASTLE_AGE,
            3 => IMPERIAL_AGE,
            _ => return now,
        };
        self.event_of_kind(kind)
            .map_or(now, |e| now.wrapping_sub(e.world_time / 1000))
    }

    /// `update` (`FUN_141569450`), once a game second with the player's `attributes`, the world's
    /// time and the history period (`world+0x8b8`, the `.dat`'s `time_slice`). Every period:
    /// - an entry of the civilian and military population (attributes 37 and 40 as shorts; both
    ///   0 when their sum is below 0),
    /// - the kills and razings since the last period against the player's value at the last
    ///   period (attributes 164 and 165): the share of its own kills' and razings' value
    ///   (attributes 170 and 172) at 0.12 or more ([`BATTLE_START`], as doubles) starts or goes
    ///   on with a battle, which sums them and the share of all four (with 152 and 153); a
    ///   battle that falls below it ends, recorded as an event when its summed share reached
    ///   0.15 ([`BATTLE_RECORD`]) with the opponent whose razings' value (attribute 425 + i)
    ///   over the player's value plus kills' value (400 + i) since the battle began is the
    ///   largest (the first of equals, none above 0 gives player 0),
    /// - and the attributes are kept for the next period, the per-player ones outside a battle.
    pub fn update(&mut self, attributes: &[f32], world_time: u32, period: i32) {
        self.update_count = self.update_count.wrapping_add(1);
        if self.update_count < period {
            return;
        }
        self.update_count = self.update_count.wrapping_sub(period);
        // the save holds every attribute of the .dat's count, these among them
        let a = |i: usize| attributes.get(i).copied().unwrap_or(0.0);
        let int = |i: usize| cvttss2si(a(i));
        let civilian = a(attribute::CIVILIAN_POPULATION);
        let military = a(attribute::MILITARY_POPULATION);
        if military + civilian < 0.0 {
            self.add_entry(0, 0.0, 0.0);
        } else {
            self.add_entry(0, short(civilian), short(military));
        }
        let kills = int(attribute::KILLS).wrapping_sub(self.old_total_kills);
        let kills_value = int(attribute::KILLS_VALUE).wrapping_sub(self.old_build_value_kills);
        let razings = int(attribute::RAZINGS).wrapping_sub(self.old_total_razings);
        let razings_value =
            int(attribute::RAZINGS_VALUE).wrapping_sub(self.old_build_value_razings);
        let kills_other = int(attribute::KILLS_OTHER).wrapping_sub(self.old_total_kills_other);
        let kills_value_other =
            int(attribute::KILLS_VALUE_OTHER).wrapping_sub(self.old_build_value_kills_other);
        let razings_other =
            int(attribute::RAZINGS_OTHER).wrapping_sub(self.old_total_razings_other);
        let razings_value_other =
            int(attribute::RAZINGS_VALUE_OTHER).wrapping_sub(self.old_build_value_razings_other);
        let value = self
            .old_current_build_value_buildings
            .wrapping_add(self.old_current_build_value_units);
        let (share, total_share) = if value < 1 {
            (0.0, 0.0)
        } else {
            let own = razings_value.wrapping_add(kills_value);
            let all = razings_value_other
                .wrapping_add(kills_value_other)
                .wrapping_add(razings_value)
                .wrapping_add(kills_value);
            (own as f32 / value as f32, all as f32 / value as f32)
        };
        let battling = f64::from(share) >= BATTLE_START;
        let goes_on = if self.battle_flag == 0 {
            if battling {
                self.battle_flag = 1;
            }
            battling
        } else if battling {
            true
        } else {
            if f64::from(self.running_ave_bv_percent) >= BATTLE_RECORD {
                self.record_battle(attributes, value, world_time);
            }
            self.battle_flag = 0;
            self.running_total_bv_kills = 0;
            self.running_total_bv_razings = 0;
            self.running_total_kills = 0;
            self.running_total_razings = 0;
            self.running_ave_bv_percent = 0.0;
            false
        };
        if goes_on {
            self.running_total_bv_kills = self
                .running_total_bv_kills
                .wrapping_add(kills_value_other.wrapping_add(kills_value));
            self.running_total_bv_razings = self
                .running_total_bv_razings
                .wrapping_add(razings_value_other.wrapping_add(razings_value));
            self.running_total_kills = self
                .running_total_kills
                .wrapping_add(i32::from(kills as u16).wrapping_add(i32::from(kills_other as u16)));
            self.running_total_razings = self.running_total_razings.wrapping_add(
                i32::from(razings as u16).wrapping_add(i32::from(razings_other as u16)),
            );
            // total_share + running in the game; the share is never a NaN, so the order is moot
            self.running_ave_bv_percent += total_share;
        }
        self.old_total_razings = int(attribute::RAZINGS);
        self.old_build_value_razings = int(attribute::RAZINGS_VALUE);
        self.old_total_razings_other = int(attribute::RAZINGS_OTHER);
        self.old_build_value_razings_other = int(attribute::RAZINGS_VALUE_OTHER);
        self.old_total_kills = int(attribute::KILLS);
        self.old_build_value_kills = int(attribute::KILLS_VALUE);
        self.old_total_kills_other = int(attribute::KILLS_OTHER);
        self.old_build_value_kills_other = int(attribute::KILLS_VALUE_OTHER);
        self.old_current_build_value_units = int(attribute::UNITS_VALUE);
        self.old_current_build_value_buildings = int(attribute::BUILDINGS_VALUE);
        if self.battle_flag == 0 {
            for i in 0..self.old_player_kill_bv.len() {
                self.old_player_kill_bv[i] = cvttss2si_64(a(KILL_VALUE_OF + i)) as i32;
                if let Some(r) = self.old_player_razing_bv.get_mut(i) {
                    *r = cvttss2si_64(a(RAZING_VALUE_OF + i)) as i32;
                }
            }
        }
    }

    /// The end of a battle worth recording (`FUN_141569450` from `0x141569a2a`): each player's
    /// kills' and razings' value since the battle began (`CVTTSS2SI` into 64 bits, the low 32
    /// kept), the opponent and the event, with the summed share as its `data2`.
    fn record_battle(&mut self, attributes: &[f32], value: i32, world_time: u32) {
        let a = |i: usize| attributes.get(i).copied().unwrap_or(0.0);
        let players = self.old_player_kill_bv.len();
        let kills: Vec<i32> = (0..players)
            .map(|i| {
                (cvttss2si_64(a(KILL_VALUE_OF + i)) as i32).wrapping_sub(self.old_player_kill_bv[i])
            })
            .collect();
        let razings: Vec<i32> = (0..players)
            .map(|i| {
                let old = self.old_player_razing_bv.get(i).copied().unwrap_or(0);
                (cvttss2si_64(a(RAZING_VALUE_OF + i)) as i32).wrapping_sub(old)
            })
            .collect();
        let mut best = 0.0f32;
        let mut opponent = 0i32;
        for i in 0..players {
            let weight = razings[i] as f32 / value as f32 + kills[i] as f32;
            if weight > best {
                best = weight;
                opponent = i as i32;
            }
        }
        if self.entries.is_empty() {
            return;
        }
        self.events.insert(
            0,
            Event {
                kind: BATTLE,
                time_slice: self.entries.len() as i32,
                world_time,
                data1: opponent as f32,
                data2: self.running_ave_bv_percent,
                data3: 0.0,
            },
        );
        let _ = self.mark_new_battle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battle(data2: f32) -> Event {
        Event {
            kind: BATTLE,
            data2,
            ..Event::default()
        }
    }

    fn with_entry() -> History {
        History {
            entries: vec![[0, 0]],
            max_entries: 60,
            ..History::default()
        }
    }

    /// `add_new_history_entry`: a full buffer grows by 60 first; an entry of kind 0 holds the
    /// values through `CVTTSS2SI` as shorts, another kind leaves zeros, and NaN is the integer
    /// indefinite's low half, 0.
    #[test]
    fn an_entry_grows_the_buffer_by_sixty_and_keeps_shorts() {
        let mut h = History::default();
        h.add_entry(0, 7.9, 70_000.0);
        assert_eq!(
            (h.max_entries, h.entries.clone()),
            (60, vec![[7, 70_000_i32 as i16]])
        );
        h.add_entry(1, 3.0, 4.0);
        h.add_entry(0, f32::NAN, -2.5);
        assert_eq!(h.entries, vec![[7, 4464], [0, 0], [0, -2]]);
        h.entries.resize(60, [0, 0]);
        h.add_entry(0, 1.0, 1.0);
        assert_eq!((h.max_entries, h.entries.len()), (120, 61));
    }

    /// `add_history_event`: nothing without an entry; else the event at the head, at the
    /// entries' count and the world's time, its byte as `data3`.
    #[test]
    fn an_event_needs_an_entry_and_joins_the_head() {
        let mut h = History::default();
        h.add_event(CASTLE_BUILT, 2.0, 0.0, 0, 5000);
        assert_eq!(h.events, []);
        h.entries = vec![[0, 0]; 3];
        h.add_event(FEUDAL_AGE, 0.0, 0.0, 0, 5000);
        h.add_event(CASTLE_BUILT, 2.0, 0.5, 7, 9000);
        assert_eq!(
            h.events,
            vec![
                Event {
                    kind: CASTLE_BUILT,
                    time_slice: 3,
                    world_time: 9000,
                    data1: 2.0,
                    data2: 0.5,
                    data3: 7.0,
                },
                Event {
                    kind: FEUDAL_AGE,
                    time_slice: 3,
                    world_time: 5000,
                    ..Event::default()
                },
            ]
        );
        assert_eq!(h.number_of_events, 0);
    }

    /// `mark_new_battle`: up to three battles all stay (the new one's `data3` 1); a fourth
    /// pushes out the battle among the three newest furthest below it, or goes itself.
    #[test]
    fn a_fourth_battle_pushes_out_the_weakest_of_the_newest_three() {
        let mut h = with_entry();
        for d in [0.2, 0.9, 0.3] {
            h.add_event(BATTLE, 1.0, d, 0, 0);
        }
        assert_eq!(h.events.len(), 3);
        assert!(h.events.iter().all(|e| e.data3 == 1.0));
        // the newest three are 0.5 (new), 0.3, 0.9: 0.3 lies furthest below
        h.add_event(BATTLE, 1.0, 0.5, 0, 0);
        let d: Vec<f32> = h.events.iter().map(|e| e.data2).collect();
        assert_eq!(d, vec![0.5, 0.9, 0.2]);
        // a new battle below the newest three goes itself; 0.2 is fourth and not compared
        h.events.insert(
            0,
            Event {
                kind: FEUDAL_AGE,
                ..Event::default()
            },
        );
        h.add_event(BATTLE, 1.0, 0.4, 0, 0);
        assert_eq!(h.events.iter().filter(|e| e.kind == BATTLE).count(), 3);
        assert_eq!(h.events[0].kind, FEUDAL_AGE);
        // a NaN gap never qualifies
        h.events.insert(0, battle(f32::NAN));
        assert!(!h.mark_new_battle());
        assert_eq!(h.events.len(), 4);
    }

    /// `mark_new_battle` scans the newest battles in list order: the first of two equal gaps
    /// wins, and other events between them do not count.
    #[test]
    fn equal_gaps_push_out_the_newer_battle() {
        let mut h = with_entry();
        h.events = vec![battle(0.1), Event::default(), battle(0.1), battle(0.9)];
        h.add_event(BATTLE, 3.0, 0.6, 0, 0);
        let d: Vec<f32> = h.events.iter().map(|e| e.data2).collect();
        assert_eq!(d, vec![0.6, 0.0, 0.1, 0.9]);
    }

    /// `FUN_141560e20` and `get_history_event_by_time`: the newest of a kind; the first position
    /// (1 based) and count of a time slice's events.
    #[test]
    fn the_readers_find_the_newest_event_and_a_time_slice_s_events() {
        let h = History {
            events: vec![
                Event {
                    kind: CASTLE_AGE,
                    time_slice: 4,
                    world_time: 9000,
                    ..Event::default()
                },
                Event {
                    kind: FEUDAL_AGE,
                    time_slice: 2,
                    world_time: 4000,
                    ..Event::default()
                },
                Event {
                    kind: FEUDAL_AGE,
                    time_slice: 2,
                    world_time: 1000,
                    ..Event::default()
                },
            ],
            ..History::default()
        };
        assert_eq!(
            h.event_of_kind(FEUDAL_AGE).map(|e| e.world_time),
            Some(4000)
        );
        assert_eq!(h.event_of_kind(IMPERIAL_AGE), None);
        assert_eq!(h.events_at(2), Some((2, 2)));
        assert_eq!(h.events_at(4), Some((1, 1)));
        assert_eq!(h.events_at(3), None);
    }

    /// `FUN_140b077d0`: the seconds since the current age's event, else the world's seconds.
    #[test]
    fn the_age_time_counts_from_the_age_s_event() {
        let h = History {
            events: vec![Event {
                kind: CASTLE_AGE,
                world_time: 600_999,
                ..Event::default()
            }],
            ..History::default()
        };
        assert_eq!(h.age_time(2.0, 900_500), 300);
        assert_eq!(h.age_time(1.9, 900_500), 900);
        assert_eq!(h.age_time(3.0, 900_500), 900);
        assert_eq!(h.age_time(0.0, 900_500), 900);
    }

    fn attributes() -> Vec<f32> {
        vec![0.0; 450]
    }

    /// The update counts its runs: the period's run adds the entry and keeps the attributes.
    #[test]
    fn the_period_s_update_adds_an_entry_and_keeps_the_attributes() {
        let mut h = History {
            update_count: 28,
            old_player_kill_bv: vec![0; 3],
            old_player_razing_bv: vec![0; 3],
            ..History::default()
        };
        let mut a = attributes();
        a[attribute::CIVILIAN_POPULATION] = 12.0;
        a[attribute::MILITARY_POPULATION] = 3.0;
        a[attribute::KILLS] = 2.0;
        a[attribute::UNITS_VALUE] = 500.0;
        a[attribute::BUILDINGS_VALUE] = 900.7;
        a[KILL_VALUE_OF + 2] = 45.0;
        a[RAZING_VALUE_OF + 1] = 5_000_000_000.0;
        h.update(&a, 0, 30);
        assert_eq!((h.update_count, h.entries.len()), (29, 0));
        h.update(&a, 0, 30);
        assert_eq!((h.update_count, h.entries.clone()), (0, vec![[12, 3]]));
        assert_eq!(
            (h.old_total_kills, h.old_current_build_value_buildings),
            (2, 900)
        );
        assert_eq!(h.old_player_kill_bv, vec![0, 0, 45]);
        // 5e9 through the 64-bit conversion keeps its low half
        assert_eq!(h.old_player_razing_bv, vec![0, 5_000_000_000_i64 as i32, 0]);
        // a negative population sum gives an entry of zeros
        a[attribute::MILITARY_POPULATION] = -13.0;
        h.update_count = 29;
        h.update(&a, 0, 30);
        assert_eq!(h.entries[1], [0, 0]);
    }

    /// A share of the player's value of 0.12 or more starts a battle and sums it; falling below
    /// ends it, with an event when the summed share reached 0.15, against the player whose
    /// razings over the value plus kills since the start weigh most.
    #[test]
    fn a_battle_starts_sums_and_ends_with_its_event() {
        let mut h = History {
            update_count: 29,
            old_current_build_value_units: 600,
            old_current_build_value_buildings: 400,
            old_player_kill_bv: vec![0; 3],
            old_player_razing_bv: vec![0; 3],
            ..History::default()
        };
        let mut a = attributes();
        a[attribute::UNITS_VALUE] = 600.0;
        a[attribute::BUILDINGS_VALUE] = 400.0;
        a[attribute::KILLS_VALUE] = 100.0;
        a[attribute::RAZINGS_VALUE] = 30.0;
        a[attribute::KILLS_VALUE_OTHER] = 30.0;
        a[attribute::KILLS] = 70_000.0;
        a[attribute::KILLS_OTHER] = 3.0;
        a[attribute::RAZINGS] = 1.0;
        a[KILL_VALUE_OF + 1] = 50.0;
        a[KILL_VALUE_OF + 2] = 40.0;
        a[RAZING_VALUE_OF + 2] = 20_000.0;
        // (30 + 100) / 1000: the battle starts; all four: 160 / 1000
        h.update(&a, 30_000, 30);
        assert_eq!(h.battle_flag, 1);
        assert_eq!(
            (h.running_total_bv_kills, h.running_total_bv_razings),
            (130, 30)
        );
        assert_eq!(
            (h.running_total_kills, h.running_total_razings),
            (4464 + 3, 1)
        );
        assert_eq!(h.running_ave_bv_percent, 160.0 / 1000.0);
        // in a battle the per-player values stay as they were
        assert_eq!(h.old_player_kill_bv, vec![0, 0, 0]);
        // nothing more: the share falls to 0, the battle ends with its event; player 2's
        // 20000 / 1000 + 40 outweighs player 1's 50
        h.update_count = 29;
        h.update(&a, 60_000, 30);
        assert_eq!(h.battle_flag, 0);
        assert_eq!(h.running_ave_bv_percent, 0.0);
        assert_eq!(
            h.events,
            vec![Event {
                kind: BATTLE,
                time_slice: 2,
                world_time: 60_000,
                data1: 2.0,
                data2: 160.0 / 1000.0,
                data3: 1.0,
            }]
        );
        assert_eq!(h.old_player_kill_bv, vec![0, 50, 40]);
        assert_eq!(h.old_player_razing_bv, vec![0, 0, 20_000]);
    }

    /// A battle whose summed share stays below 0.15 ends without an event; a saved battle flag
    /// other than 1 goes on as a battle.
    #[test]
    fn a_small_battle_ends_without_its_event() {
        let mut h = History {
            update_count: 29,
            battle_flag: 2,
            running_ave_bv_percent: 0.149_999,
            old_current_build_value_units: 1000,
            old_player_kill_bv: vec![0; 2],
            old_player_razing_bv: vec![0; 2],
            ..History::default()
        };
        let mut a = attributes();
        a[attribute::UNITS_VALUE] = 1000.0;
        a[attribute::KILLS_VALUE] = 130.0;
        h.update(&a, 0, 30);
        assert_eq!(h.battle_flag, 2);
        h.update_count = 29;
        h.running_ave_bv_percent = 0.149_999;
        h.update(&a, 0, 30);
        assert_eq!((h.battle_flag, h.events.len()), (0, 0));
        // a NaN sum fails the record's COMISD (JC): no event either
        h.update_count = 29;
        h.battle_flag = 1;
        h.running_ave_bv_percent = f32::NAN;
        h.update(&a, 0, 30);
        assert_eq!((h.battle_flag, h.events.len()), (0, 0));
        assert_eq!(h.running_ave_bv_percent, 0.0);
    }

    /// The share is a float compared as a double: 120 of 1000 as an f32 lies just below the
    /// double 0.12 and starts no battle.
    #[test]
    fn a_share_of_exactly_twelve_percent_starts_no_battle() {
        let mut h = History {
            update_count: 29,
            old_current_build_value_units: 1000,
            ..History::default()
        };
        let mut a = attributes();
        a[attribute::UNITS_VALUE] = 1000.0;
        a[attribute::KILLS_VALUE] = 120.0;
        h.update(&a, 0, 30);
        assert_eq!(h.battle_flag, 0);
    }
}
