//! The effects' `do_effect` (`FUN_141462440`, then the base `FUN_140ec2ac0`): each command by
//! its kind for every party ([`Party`]).

use crate::GenieError;
use crate::objects::masters::ClassId;
use crate::players::PlayerId;
use crate::technology::EffectCommand;
use crate::technology::effect::{
    COST_ATTRIBUTES, Change, FARM_FOOD, TECH_COST, TECH_DISABLE, TECH_TIME, kind,
};
use crate::world::World;

/// Which players a block of the base's kinds reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Party {
    /// The player itself (0 to 8).
    Own,
    /// Its mutual allies (10 to 18, both relations 0).
    Allies,
    /// Its enemies (20 to 28, both relations 3 or both 4).
    Enemies,
    /// Its neutrals (30 to 38, both relations 1).
    Neutrals,
    /// Gaia (40 to 48).
    Gaia,
}

impl Party {
    /// The party of a command kind's block (its tens: 0 the player, 1 its allies, 2 its enemies,
    /// 3 its neutrals, 4 Gaia).
    pub(super) const fn of_block(block: u8) -> Self {
        match block {
            0 => Self::Own,
            1 => Self::Allies,
            2 => Self::Enemies,
            3 => Self::Neutrals,
            _ => Self::Gaia,
        }
    }
}

impl World {
    /// The players party `party` reaches from player `p`: for a relation, each player past Gaia
    /// (`FUN_14152fdd0`, from 1, the player itself when it counts itself so) whose and to whom
    /// the relations (`+0x260`, `FUN_141667f50`) are the party's, in order.
    pub(super) fn party(&self, p: PlayerId, party: Party) -> Vec<PlayerId> {
        let relation = |a: PlayerId, b: PlayerId| {
            self.players
                .get(a.index())?
                .relations
                .get(b.index())
                .copied()
        };
        let mutual = |q: PlayerId, wanted: &[u8]| {
            wanted
                .iter()
                .any(|&r| relation(p, q) == Some(r) && relation(q, p) == Some(r))
        };
        let wanted: &[u8] = match party {
            Party::Own => return vec![p],
            Party::Gaia => return vec![PlayerId::GAIA],
            Party::Allies => &[0],
            Party::Enemies => &[3, 4],
            Party::Neutrals => &[1],
        };
        PlayerId::each(self.players.len())
            .skip(1)
            .filter(|&q| mutual(q, wanted))
            .collect()
    }

    /// `TRIBE_Effects::do_effect` (`FUN_141462440`) of an effect's commands `effect` (the
    /// effect's id names them in the game's table; its 0x318 makes the spawns count in the local
    /// player's statistics, which are not simulation) for player `p` with the effect's object `object` (none for a research's): first its
    /// technology commands in order, the cost modifier (`'e'`, [`World::tech_cost`]), the disable
    /// command (`'f'`, an id other than −1, [`World::disable_tech`]) and the research time
    /// modifier (`'g'`, [`World::research_time`]); then the base's ([`World::base_effect`]).
    pub(crate) fn do_effect(
        &mut self,
        p: PlayerId,
        effect: &[EffectCommand],
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        for c in effect {
            // the technology commands read the third argument's low byte as the mode
            let mode = (c.c & 0xff) as u8;
            match c.kind {
                TECH_COST => self.tech_cost(p, c.a, c.b, c.d, mode)?,
                TECH_DISABLE => {
                    let tech = crate::math::conversions::cvttss2si(c.d);
                    if tech != -1 {
                        self.disable_tech(p, tech, object)?;
                    }
                }
                TECH_TIME => self.research_time(p, c.a, c.d, mode, -1)?,
                _ => {}
            }
        }
        self.base_effect(p, effect, object)
    }

    /// `RGE_Effects::do_effect` (`FUN_140ec2ac0`): each command by its kind ([`kind`]), for the
    /// players of its block ([`Party`]); a kind it does not know (the technology commands among
    /// them) does nothing. The command's third argument's low byte is the attribute of a change
    /// (slot 0x148, [`World::modify_attribute`]). After the commands, the player's change of the
    /// farms' food (resource 0x24) grows the food its farms hold ([`World::farm_food`]), by the
    /// last such command's value before and amount.
    pub(super) fn base_effect(
        &mut self,
        p: PlayerId,
        effect: &[EffectCommand],
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        let mut farms: Option<(f32, f32)> = None;
        for c in effect {
            if kind::OBJECTS_SET <= c.kind && c.kind <= kind::OBJECT_UPGRADE {
                Self::object_effect(c, object)?;
                continue;
            }
            if 4 < c.kind / 10 || 8 < c.kind % 10 {
                continue;
            }
            let party = Party::of_block(c.kind / 10);
            match c.kind % 10 {
                kind::SET | kind::ADD | kind::MULTIPLY => self.attribute_command(p, party, c)?,
                kind::RESOURCE | kind::RESOURCE_MULTIPLY => {
                    self.resource_command(p, party, c, &mut farms)?;
                }
                kind::ENABLE | kind::UPGRADE | kind::SPAWN | kind::TECH => {
                    self.master_command(p, party, c, object)?;
                }
                // 9, 19, 29, 39 are no kind
                _ => {}
            }
        }
        if let Some((before, amount)) = farms {
            self.farm_food(p, before, amount)?;
        }
        Ok(())
    }

    /// [`World::base_effect`]'s attribute changes (kinds 0, 4 and 5 of a block): set and add by
    /// slot 0x148 ([`World::modify_attribute`]) for each player of the block; multiply too, but
    /// a cost attribute by a negative amount (`COMISS` 0, d; `JBE`: not a NaN) changes the cost
    /// entries instead, except in Gaia's block (`FUN_141666ee0`).
    pub(super) fn attribute_command(
        &mut self,
        p: PlayerId,
        party: Party,
        c: &EffectCommand,
    ) -> Result<(), GenieError> {
        let attribute = (c.c & 0xff) as u8;
        let change = match c.kind % 10 {
            kind::SET => Change::Set,
            kind::ADD => Change::Add,
            _ => Change::Multiply,
        };
        let cost = change == Change::Multiply
            && COST_ATTRIBUTES.contains(&c.c)
            && 0.0 > c.d
            && party != Party::Gaia;
        for q in self.party(p, party) {
            if cost {
                let unit = if c.b < 0 { c.a } else { c.b };
                self.cost_entries(q, unit, c.c, crate::math::conversions::cvttss2si(c.d));
            } else {
                self.modify_attribute(q, change, c.a, ClassId::of_field(c.b)?, c.d, attribute)?;
            }
        }
        Ok(())
    }

    /// [`World::base_effect`]'s resource changes: kind 1 by the amount the effect's player's
    /// attribute `c` scales ([`World::scaled_amount`]; the own block's farm food is noted in
    /// `farms`, its value before and the amount, else a resource below 0 is skipped), kind 6 a
    /// multiply of a resource 0 or above; each for the block's players.
    pub(super) fn resource_command(
        &mut self,
        p: PlayerId,
        party: Party,
        c: &EffectCommand,
        farms: &mut Option<(f32, f32)>,
    ) -> Result<(), GenieError> {
        if c.kind % 10 == kind::RESOURCE_MULTIPLY {
            if 0 <= c.a {
                for q in self.party(p, party) {
                    self.multiply_resource(q, c.a, c.d)?;
                }
            }
            return Ok(());
        }
        // the effect's player's attribute `c` scales the amount
        let amount = self.scaled_amount(p, c);
        if party == Party::Own && c.a == FARM_FOOD {
            *farms = Some((self.attribute_of(p, FARM_FOOD), amount));
        } else if c.a < 0 {
            return Ok(());
        }
        let mode = (c.b & 0xff) as u8;
        for q in self.party(p, party) {
            self.modify_resource(q, c.a, amount, mode)?;
        }
        Ok(())
    }

    /// [`World::base_effect`]'s master commands for the block's players: the enable (a master 0
    /// or above; the own block's with its one-object flag and the effect's object), the upgrade
    /// (the own block's with the effect's object), the spawn and the technology change.
    pub(super) fn master_command(
        &mut self,
        p: PlayerId,
        party: Party,
        c: &EffectCommand,
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        let own = party == Party::Own;
        match c.kind % 10 {
            kind::ENABLE => {
                if 0 <= c.a {
                    let enable = (c.b & 0xff) as u8;
                    for q in self.party(p, party) {
                        let (one, object) = if own {
                            (c.c == 1, object)
                        } else {
                            (false, None)
                        };
                        self.enable_master(q, c.a, enable, one, object)?;
                    }
                }
            }
            kind::UPGRADE => {
                for q in self.party(p, party) {
                    let object = object.filter(|_| own);
                    self.upgrade_master(q, c.a, c.b, c.c, object)?;
                }
            }
            kind::SPAWN => {
                for q in self.party(p, party) {
                    self.spawn_at(q, c.a, c.b, c.c)?;
                }
            }
            _ => {
                for q in self.party(p, party) {
                    self.modify_tech(q, c.a, c.b, c.d, c.c)?;
                }
            }
        }
        Ok(())
    }

    /// A resource command's amount (`FUN_140ec2ac0`'s kinds 1, 11, ...): its value times the
    /// effect's player's attribute named by its third argument, when the player has it.
    pub(super) fn scaled_amount(&self, p: PlayerId, c: &EffectCommand) -> f32 {
        usize::try_from(c.c)
            .ok()
            .and_then(|i| self.attribute_value(p, i))
            .map_or(c.d, |scale| c.d * scale)
    }

    /// Player `p`'s attribute `i` (the resources are [`crate::world::Player::resources`]), None
    /// past its count (`+0x90`, the saved attributes').
    pub(super) fn attribute_value(&self, p: PlayerId, i: usize) -> Option<f32> {
        let player = self.players.get(p.index())?;
        let value = player.attributes.get(i).copied()?;
        Some(if i < 4 {
            *player.resources.get(i)?
        } else {
            value
        })
    }

    /// Player `p`'s attribute `i`, 0 past its count.
    pub(super) fn attribute_of(&self, p: PlayerId, i: i16) -> f32 {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.attribute_value(p, i))
            .unwrap_or(0.0)
    }
}
