//! Gathering resources: the Gather and Hunt actions with each gatherer's part of its update, a
//! farm's food, the searches for a resource, the drop sites and drop-offs, the mill's and the dock's
//! queues and the pastures.

pub(crate) mod closest_resource;
pub(crate) mod drop_off;
pub(crate) mod drop_sites;
pub mod farm_queue;
pub mod gather;
pub mod gather_search;
pub mod hunt;
pub mod make;
pub mod pasture;
