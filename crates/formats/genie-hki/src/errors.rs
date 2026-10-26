//! The errors of binding or reaching a hotkey or a group that does not exist.

/// Represents an error when binding or unbinding a hotkey that doesn't exist.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum IndexError {
    /// Represents an index error when accessing a nonexistent group.
    #[error(transparent)]
    GroupIndex(#[from] GroupIndexError),

    /// Represents an index error when accessing a nonexistent hotkey within a
    /// group.
    #[error(transparent)]
    HotkeyIndex(#[from] HotkeyIndexError),
}

/// Represents an error when accessing a hotkey group that does not exist.
///
/// The first index represents the index of the group, and the second index
/// represents the number of groups. The first index must be greater than or
/// equal to the second index.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("Group id {} must be less than the number of groups {}.", .index, .num_groups)]
pub struct GroupIndexError {
    /// The index of the group, must be greater than or equal to `num_groups`.
    pub(super) index: usize,

    /// The number of groups, must be less than or equal to `index`.
    pub(super) num_groups: usize,
}

impl GroupIndexError {
    /// Returns a `GroupIndexError` with group index `index` and a number of
    /// groups equal to `num_groups`.
    ///
    /// # Panics
    ///
    /// Panics if `index < num_groups`.
    #[must_use]
    pub fn new(index: usize, num_groups: usize) -> Self {
        assert!(num_groups <= index);
        Self { index, num_groups }
    }

    /// Returns the index of the group that was accessed.
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Returns the number of valid groups.
    #[must_use]
    pub const fn num_groups(&self) -> usize {
        self.num_groups
    }
}

/// Represents an error when accessing a hotkey that does not exist.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("Hotkey id {} must be less than the number of hotkeys {}.", .index, .num_hotkeys)]
pub struct HotkeyIndexError {
    /// The index of the hotkey, must be greater than or equal to `num_hotkeys`.
    pub(super) index: usize,

    /// The number of hotkeys, must be less than or equal to `index`.
    pub(super) num_hotkeys: usize,
}

impl HotkeyIndexError {
    /// Returns a `HotkeyIndexError` with hotkey index `index` and a number of
    /// hotkeys equal to `num_hotkeys`.
    ///
    /// # Panics
    ///
    /// Panics if `index < num_hotkeys`.
    #[must_use]
    pub fn new(index: usize, num_hotkeys: usize) -> Self {
        assert!(num_hotkeys <= index);
        Self { index, num_hotkeys }
    }

    /// Returns the index of the hotkey that was accessed.
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Returns the number of valid hotkeys.
    #[must_use]
    pub const fn num_hotkeys(&self) -> usize {
        self.num_hotkeys
    }
}
