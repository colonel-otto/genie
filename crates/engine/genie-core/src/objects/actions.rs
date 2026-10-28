//! The object's action list (`RGE_Action_List`, FD-045-D12): the actions it holds, its current
//! one (the head) first, each a variant of [`Action`].
//!
//! The game keeps a linked list at the action object's `+0x880` (its head at the list's
//! `+0x10`): only the head runs (slot 0x30, `FUN_140a992b0`), the others get the idle update
//! (`FUN_140a78470`), and a removal (`FUN_140a98e20`) tears the head down and starts the next one
//! (slot 0x28). Each kind has its accessors here (`attack()`, `attack_mut()`, `take_attack()`,
//! `set_attack()`), so a part of the object borrows the list apart from its [`Body`].
//!
//! [`Body`]: crate::objects::layers::Body

use crate::combat::attack::Attack;
use crate::combat::missile::MissileAction;
use crate::construction::build::BuildAction;
use crate::construction::repair::RepairAction;
use crate::construction::wait_solidify::WaitSolidify;
use crate::garrison::EnterAction;
use crate::garrison::transport::TransportAction;
use crate::gathering::gather::GatherAction;
use crate::gathering::hunt::HuntAction;
use crate::gathering::make::MakeAction;
use crate::movement::explore::ExploreAction;
use crate::movement::fly::Fly;
use crate::movement::move_to::MoveToAction;
use crate::ownership::artifact::Artifact;

/// Declares the action kinds: the [`Action`] variants and, for each, the list's accessors.
macro_rules! action_kinds {
    ($($(#[$doc:meta])* $variant:ident($ty:ty) => $get:ident, $get_mut:ident, $take:ident,
        $set:ident, $with:ident;)*) => {
        /// One action of an object's list, by its kind.
        #[derive(Clone, Debug, PartialEq)]
        pub enum Action {
            $($(#[$doc])* $variant($ty),)*
        }

        impl ActionList {
            $(
                #[doc = concat!("The list's `", stringify!($variant), "`, if it holds one.")]
                #[must_use]
                pub fn $get(&self) -> Option<&$ty> {
                    self.0.iter().find_map(|a| match a {
                        Action::$variant(x) => Some(x),
                        _ => None,
                    })
                }

                #[doc = concat!("The list's `", stringify!($variant), "` to change, if it holds one.")]
                pub fn $get_mut(&mut self) -> Option<&mut $ty> {
                    self.0.iter_mut().find_map(|a| match a {
                        Action::$variant(x) => Some(x),
                        _ => None,
                    })
                }

                #[doc = concat!("The list's `", stringify!($variant), "` leaves it (no teardown runs).")]
                pub fn $take(&mut self) -> Option<$ty> {
                    let i = self.0.iter().position(|a| matches!(a, Action::$variant(_)))?;
                    match self.0.remove(i) {
                        Action::$variant(x) => Some(x),
                        _ => None,
                    }
                }

                #[doc = concat!("The list's `", stringify!($variant), "` becomes `action`: in its place")]
                #[doc = "if it holds one, else at the head; None takes it out ([`ActionList::put`])."]
                pub fn $set(&mut self, action: Option<$ty>) {
                    match action {
                        Some(x) => self.put(Action::$variant(x)),
                        None => self.0.retain(|a| !matches!(a, Action::$variant(_))),
                    }
                }

                #[doc = concat!("The list with its `", stringify!($variant), "` set ([`ActionList::")]
                #[doc = concat!(stringify!($set), "`]).")]
                #[must_use]
                pub fn $with(mut self, action: Option<$ty>) -> Self {
                    self.$set(action);
                    self
                }
            )*
        }
    };
}

action_kinds! {
    /// The livestock default action ([`crate::ownership::artifact`]).
    Artifact(Artifact) => artifact, artifact_mut, take_artifact, set_artifact, with_artifact;
    /// A builder's Build ([`crate::construction::build`]).
    Build(BuildAction) => build, build_mut, take_build, set_build, with_build;
    /// A repairer's Repair ([`crate::construction::repair`]).
    Repair(RepairAction) => repair, repair_mut, take_repair, set_repair, with_repair;
    /// A `MoveTo` as the object's own action (a wandering animal).
    MoveTo(MoveToAction) => move_to, move_to_mut, take_move_to, set_move_to, with_move_to;
    /// A bird's Fly ([`crate::movement::fly`]).
    Fly(Fly) => fly, fly_mut, take_fly, set_fly, with_fly;
    /// A villager's Hunt ([`crate::gathering::hunt`]).
    Hunt(HuntAction) => hunt, hunt_mut, take_hunt, set_hunt, with_hunt;
    /// A forager's Gather ([`crate::gathering::gather`]).
    Gather(GatherAction) => gather, gather_mut, take_gather, set_gather, with_gather;
    /// Its own Attack (type 9, not a Hunt's): a boar's on its attacker.
    Attack(Attack) => attack, attack_mut, take_attack, set_attack, with_attack;
    /// An auto scout's Explore ([`crate::movement::explore`]).
    Explore(ExploreAction) => explore, explore_mut, take_explore, set_explore, with_explore;
    /// A unit's Enter (a garrison, [`crate::garrison`]).
    Enter(EnterAction) => enter, enter_mut, take_enter, set_enter, with_enter;
    /// A unit's Transport (an unload command's, [`crate::garrison::transport`]).
    Transport(TransportAction) => transport, transport_mut, take_transport, set_transport,
        with_transport;
    /// A missile's flight ([`crate::combat::missile`]).
    Missile(MissileAction) => missile, missile_mut, take_missile, set_missile, with_missile;
    /// A farm's or fish trap's Make ([`crate::gathering::make`]).
    Make(MakeAction) => make, make_mut, take_make, set_make, with_make;
    /// A villager's wait for a foundation ([`crate::construction::wait_solidify`]).
    WaitSolidify(WaitSolidify) => wait_solidify, wait_solidify_mut, take_wait_solidify,
        set_wait_solidify, with_wait_solidify;
}

/// The object's actions (`RGE_Action_List`), the head first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionList(Vec<Action>);

impl ActionList {
    /// `action` replaces the list's action of its kind in its place, or else becomes the head.
    pub fn put(&mut self, action: Action) {
        let kind = std::mem::discriminant(&action);
        match self
            .0
            .iter_mut()
            .find(|a| std::mem::discriminant(*a) == kind)
        {
            Some(held) => *held = action,
            None => self.0.insert(0, action),
        }
    }

    /// Whether the list is empty (`FUN_140a98730` false: no current action).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The current action (the head), if any.
    #[must_use]
    pub fn head(&self) -> Option<&Action> {
        self.0.first()
    }

    /// The actions, the head first.
    pub fn iter(&self) -> std::slice::Iter<'_, Action> {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a ActionList {
    type Item = &'a Action;
    type IntoIter = std::slice::Iter<'a, Action>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(state: u8) -> Artifact {
        Artifact {
            state,
            ..Artifact::new(false)
        }
    }

    fn other() -> Action {
        Action::Artifact(Artifact::new(true))
    }

    #[test]
    fn a_kind_set_twice_keeps_one_entry_in_its_place() {
        let mut list = ActionList::default();
        assert!(list.is_empty());
        list.put(Action::Artifact(artifact(1)));
        assert_eq!(list.head(), Some(&Action::Artifact(artifact(1))));
        list.set_artifact(Some(artifact(2)));
        assert_eq!(
            list.iter().cloned().collect::<Vec<_>>(),
            [Action::Artifact(artifact(2))],
            "the held kind changed in its place"
        );
        list.put(other());
        assert_eq!((&list).into_iter().count(), 1);
        assert_eq!(list.head(), Some(&other()));
    }

    #[test]
    fn the_accessors_reach_only_their_kind() {
        let mut list = ActionList::default().with_artifact(Some(artifact(7)));
        assert_eq!(list.artifact(), Some(&artifact(7)));
        assert_eq!(list.make(), None);
        if let Some(a) = list.artifact_mut() {
            a.state = 8;
        }
        assert_eq!(list.make_mut(), None);
        assert_eq!(list.take_make(), None);
        assert_eq!(list.take_artifact(), Some(artifact(8)));
        assert!(list.is_empty());
    }

    #[test]
    fn setting_none_takes_the_kind_out() {
        let mut list = ActionList::default().with_artifact(Some(artifact(1)));
        list.set_make(None);
        assert_eq!(list.iter().count(), 1);
        list.set_artifact(None);
        assert_eq!(list.artifact(), None);
        list.set_artifact(None);
        assert!(list.is_empty());
    }
}
