//! `UNIT_ORDER`'s Housing (case 3) and the work that keeps a building from its cases 1 to 3.

use crate::commands::QueueId;
use crate::objects::ListIndex;
use crate::objects::ObjectId;
use crate::objects::ObjectState;
use crate::objects::masters::ActionKind;
use crate::objects::masters::MasterKind;
use crate::orders::unit_order::ORDER_ASLEEP;
use crate::players::PlayerId;
use crate::production::Making;
use crate::world::Unread;
use crate::world::World;
use crate::world::index::Place;
use crate::world::slots::PlayerSlotsExt;
use crate::{GenieError, GenieResult};

#[cfg(test)]
mod tests;

/// The action types that keep a building from `UNIT_ORDER`'s cases 1 to 3 (the Pack, the Unpack
/// and the Housing): the order's own compares of `FUN_1411e8630`'s type at `0x14161ea3e`. The
/// building's production (`FUN_141227a00`, [`crate::production::spawn`]) waits on the same
/// types by a switch of its own.
const ORDER_BUSY: [ActionKind; 5] = [
    ActionKind::MAKE_OBJECT,
    ActionKind::MAKE_TECH,
    ActionKind::HOUSING,
    ActionKind::PACK,
    ActionKind::UNPACK,
];

/// The Housing order on an object below the buildings' type that holds less than
/// `HousingTuning::full`: the order makes the Housing for a null building (`RDI` 0) and calls its
/// slot 0x808 through it (`MOV RAX, [RDI]` at `0x14161ee8b`), where the game faults.
const HOUSING_WITHOUT_BUILDING: GenieError =
    GenieError::unreachable("a UNIT_ORDER Housing writes through a null building");

impl World {
    /// `UNIT_ORDER`'s Housing (`FUN_14161e370` case 3) to the units `ids`, with the order's queue
    /// id `queue_id`: each object the world holds in state 2 (`+0x640`), any player's, takes a new
    /// Housing ([`World::install_housing`]) when it is a building (type 80 or more) not at work
    /// ([`World::building_work_kind`] none of [`ORDER_BUSY`]) that holds no headroom (`+0x308` not
    /// 4), or when it holds less than `HousingTuning::full` (`+0x310`, `COMISS`, `JBE`: a NaN does
    /// not): a building at work or holding headroom then takes it too. An object below the
    /// buildings' type that holds less faults in the game ([`HOUSING_WITHOUT_BUILDING`]).
    ///
    /// # Errors
    ///
    /// A sleeping object, the fault, the work's and the install's.
    pub(crate) fn housing_order(
        &mut self,
        ids: &[Option<ObjectId>],
        queue_id: QueueId,
    ) -> GenieResult<()> {
        for &id in ids {
            let Some(place) = self.place(id) else {
                continue;
            };
            let Place::Live(player_id, idx) = place else {
                return Err(ORDER_ASLEEP);
            };
            let unit = self.players.live(player_id, idx)?;
            if unit.life.state != ObjectState::Alive {
                continue;
            }
            let building =
                unit.master(self.master_tables(), player_id)?.kind >= MasterKind::BUILDING;
            let busy = building
                && self
                    .building_work_kind(player_id, idx)?
                    .is_some_and(|kind| ORDER_BUSY.contains(&kind));
            let unit = self.players.live(player_id, idx)?;
            let holds_headroom =
                unit.life.attribute_kind == Some(self.model.slots.attribute.headroom);
            let below_full = self.model.tuning.housing.full > unit.life.amount.get();
            let takes = building && !busy && !holds_headroom || below_full;
            if !takes {
                continue;
            }
            if !building {
                return Err(HOUSING_WITHOUT_BUILDING.with("object", unit.id));
            }
            self.install_housing(player_id, idx, queue_id)?;
        }
        Ok(())
    }

    /// The action type `FUN_1411e8630` gives `UNIT_ORDER` (`param_2`, with `param_7` 0) for
    /// building `idx` of player `player_id`: for a computer player (`FUN_141659cf0`) its
    /// production first (`FUN_1411d5520`): the make (0x66) or research (0x67) of its production
    /// list (`+0xbf8`), else 0x66 while its queue holds an entry (`+0xbf2` above 0); else, and for
    /// any other player, its top action's type (`FUN_141227a00`, `FUN_140a986f0`), none without
    /// one (the routine's 0). The progress the routine also answers is not read here.
    ///
    /// # Errors
    ///
    /// A player or an object the world lacks, and an object the load could not read whole
    /// ([`Unread::error`]), whose actions the slice does not hold.
    pub(crate) fn building_work_kind(
        &self,
        player_id: PlayerId,
        idx: ListIndex,
    ) -> GenieResult<Option<ActionKind>> {
        let id = self.players.live(player_id, idx)?.id;
        if let Some(error) = self.parts.unread.get(id).map(Unread::error) {
            return Err(error);
        }
        if self.players.player(player_id)?.kind.is_computer()
            && let Some(production) = self.parts.production.get(id)
        {
            match production.action {
                Some(Making::Unit(_)) => return Ok(Some(ActionKind::MAKE_OBJECT)),
                Some(Making::Research(_)) => return Ok(Some(ActionKind::MAKE_TECH)),
                None if 0 < production.queue.count.get() => {
                    return Ok(Some(ActionKind::MAKE_OBJECT));
                }
                None => {}
            }
        }
        Ok(self.parts.actions_of(id).top_action_kind())
    }
}
