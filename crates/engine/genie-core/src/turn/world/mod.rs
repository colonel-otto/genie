//! Lookups and releases across the players' objects: victims, drop sites, resources, task
//! targets, deaths and ownership changes.

mod carts;
mod deaths;
mod drop_sites;
mod ownership;
mod release;
mod resource_search;
mod task_finder;

pub(in crate::turn) use self::deaths::clear_look_of;
pub(crate) use self::deaths::{
    built_death, combat_dies, count_class, farm_bonus, forget_resource, refund_of, storage_leaves,
};
#[cfg(test)]
pub(in crate::turn) use self::drop_sites::counted;
pub(crate) use self::drop_sites::{Seeker, drop_site_search};
pub(in crate::turn) use self::drop_sites::{UNPACK, nearest_drop_site};
pub(crate) use self::ownership::{DEAD_COPY_TYPES, change_hands};
pub(in crate::turn) use self::release::release_own;
pub(crate) use self::release::{Releaser, released};
#[cfg(test)]
pub(in crate::turn) use self::resource_search::search_origin;
pub(in crate::turn) use self::resource_search::{ResourceWorld, closest_resource_of, herded};
pub(crate) use self::task_finder::World;
pub(in crate::turn) use self::task_finder::{
    SideWorld, state_of, task_search, task_target_with, wait_progress,
};
#[cfg(test)]
pub(crate) use crate::combat::victims::{is_trebuchet, paid_cost};
pub(crate) use crate::combat::victims::{victim_in, victim_of};
