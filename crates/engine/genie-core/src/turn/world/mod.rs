//! Lookups and releases across the players' objects: victims, drop sites, resources, task
//! targets, deaths and ownership changes.

mod carts;
mod deaths;
mod release;

pub(crate) use self::deaths::clear_look_of;
pub(crate) use self::deaths::{
    built_death, combat_dies, count_class, farm_bonus, forget_resource, refund_of, storage_leaves,
};
pub(crate) use self::release::release_own;
pub(crate) use self::release::{Releaser, released};
#[cfg(test)]
pub(crate) use crate::combat::victims::{is_trebuchet, paid_cost};
pub(crate) use crate::combat::victims::{victim_in, victim_of};
#[cfg(test)]
pub(in crate::turn) use crate::gathering::closest_resource::search_origin;
pub(crate) use crate::gathering::closest_resource::{ResourceWorld, closest_resource_of, herded};
#[cfg(test)]
pub(in crate::turn) use crate::gathering::drop_sites::counted;
pub(crate) use crate::gathering::drop_sites::{Seeker, drop_site_search};
pub(crate) use crate::gathering::drop_sites::{UNPACK, nearest_drop_site};
pub(crate) use crate::ownership::change::{DEAD_COPY_TYPES, change_hands};
pub(crate) use crate::unit_ai::task_finder::world::World;
pub(crate) use crate::unit_ai::task_finder::world::{
    SideWorld, state_of, task_search, task_target_with, wait_progress,
};
