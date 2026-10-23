//! Reader/writer for Age of Empires 2 Hotkey info files.
//!
//! Hotkey files in `AoE2` contain groups, each of which contain some number of
//! hotkeys. Hotkeys have a string ID, a keycode, and flags
//! for Ctrl/Alt/Shift modifiers. The index of the hotkey in its
//! group determines the action that will be taken when it is activated.

#![deny(future_incompatible)]
#![deny(nonstandard_style)]
#![deny(rust_2018_idioms)]
#![deny(unsafe_code)]
#![warn(unused)]

use genie_lang::{LangFile, StringKey};

use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use flate2::{Compression, read::DeflateDecoder, write::DeflateEncoder};
use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::slice;
use std::vec;

/// Returns `Ok(id)`, where `id` is the id number of the string giving the text
/// representation of `keycode` in a language file.
/// Returns `None` if `keycode` is not represented in a language file.
#[must_use]
pub const fn keycode_id(keycode: i32) -> Option<i32> {
    match keycode {
        112 => Some(19545),
        113 => Some(19546),
        114 => Some(19547),
        115 => Some(19548),
        116 => Some(19549),
        117 => Some(19550),
        118 => Some(19551),
        119 => Some(19552),
        120 => Some(19553),
        // Note F10 is reserved for opening the menu and cannot be reassigned
        122 => Some(19555),
        123 => Some(19556),
        124 => Some(19557),
        125 => Some(19558),
        126 => Some(19559),
        127 => Some(19560),
        128 => Some(19561),
        129 => Some(19562),
        130 => Some(19563),
        131 => Some(19564),
        132 => Some(19565),
        133 => Some(19566),
        134 => Some(19567),
        135 => Some(19568),
        253 => Some(19712),
        254 => Some(19711),
        255 => Some(19710),
        _ => None,
    }
}

/// A list of information about hotkey groups in a hotkey file.
///
/// The length is the number of groups in the file.
/// Each `StringKey` in the list is the key of the string that names the group.
/// The key is stored at the group's offset index in the hotkey file.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct HotkeyInfoMetadata(Vec<StringKey>);

impl HotkeyInfoMetadata {
    /// Returns an empty `HotkeyInfoMetadata` struct.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::HotkeyInfoMetadata;
    ///
    /// let him = HotkeyInfoMetadata::new();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a group to the metadata.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::HotkeyInfoMetadata;
    /// use genie_lang::StringKey;
    ///
    /// let mut him = HotkeyInfoMetadata::new();
    /// him.add(StringKey::from(0u32));
    /// ```
    pub fn add(&mut self, sk: StringKey) {
        self.0.push(sk);
    }

    /// Returns the number of groups described by this metadata.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::HotkeyInfoMetadata;
    /// use genie_lang::StringKey;
    ///
    /// let mut him = HotkeyInfoMetadata::new();
    /// assert_eq!(0, him.len());
    /// him.add(StringKey::from(0u32));
    /// assert_eq!(1, him.len());
    /// ```
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if this metadata does not contain any groups.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::HotkeyInfoMetadata;
    /// use genie_lang::StringKey;
    ///
    /// let mut him = HotkeyInfoMetadata::new();
    /// assert!(him.is_empty());
    /// him.add(StringKey::from(0u32));
    /// assert!(!him.is_empty());
    /// ```
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns `Some(sk)`, where `sk` is the string key of the group at the
    /// given `index`.
    /// Returns `None` if no group is located at `index`, that is,
    /// if `index >= self.len()`.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::HotkeyInfoMetadata;
    /// use genie_lang::StringKey;
    ///
    /// let mut him = HotkeyInfoMetadata::new();
    /// assert_eq!(None, him.get(0));
    /// him.add(StringKey::from(0u32));
    /// assert_eq!(Some(&StringKey::from(0u32)), him.get(0));
    /// ```
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&StringKey> {
        self.0.get(index)
    }

    /// Returns an iterator over the hotkey group metadata contained in this
    /// hotkey file's data.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::HotkeyInfoMetadata;
    /// use genie_lang::StringKey;
    ///
    /// let mut him = HotkeyInfoMetadata::new();
    /// him.add(StringKey::from(0u32));
    /// him.add(StringKey::from(1u32));
    /// let mut iter = him.iter();
    /// assert_eq!(Some(&StringKey::from(0u32)), iter.next());
    /// assert_eq!(Some(&StringKey::from(1u32)), iter.next());
    /// assert_eq!(None, iter.next());
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = &StringKey> {
        self.0.iter()
    }
}

impl From<Vec<StringKey>> for HotkeyInfoMetadata {
    fn from(sks: Vec<StringKey>) -> Self {
        Self(sks)
    }
}

impl IntoIterator for HotkeyInfoMetadata {
    type Item = StringKey;
    type IntoIter = vec::IntoIter<StringKey>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a HotkeyInfoMetadata {
    type Item = &'a StringKey;
    type IntoIter = slice::Iter<'a, StringKey>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

// TODO Would like to move this to some kind of configuration file format
// then other mods can specify which hotkeys they use and what strings their
// groups should use
// and the group names can be overwritten in the string files
/// Returns a `HotkeyInfoMetadata` struct that represents the info metadata for
/// the default Aoe2 hotkeys (both `UserPatch` and HD).
#[must_use]
pub fn default_him() -> HotkeyInfoMetadata {
    let mut hgm = HotkeyInfoMetadata::new();
    hgm.add(StringKey::from(20000_u32)); // UnitCommands
    hgm.add(StringKey::from(20001_u32)); // GameCommands
    hgm.add(StringKey::from(20002_u32)); // Scroll
    hgm.add(StringKey::from(20003_u32)); // Villager
    hgm.add(StringKey::from(20004_u32)); // TownCenter
    hgm.add(StringKey::from(20007_u32)); // Dock
    hgm.add(StringKey::from(20008_u32)); // Barracks
    hgm.add(StringKey::from(20009_u32)); // ArcheryRange
    hgm.add(StringKey::from(20010_u32)); // Stable
    hgm.add(StringKey::from(20011_u32)); // SiegeWorkshop
    hgm.add(StringKey::from(20012_u32)); // Monastery
    hgm.add(StringKey::from(20013_u32)); // Market
    hgm.add(StringKey::from(20014_u32)); // MilitaryUnits
    hgm.add(StringKey::from(20015_u32)); // Castle
    hgm.add(StringKey::from(20017_u32)); // Mill
    hgm
}

// TODO PCM III uses lots of hard coded changes for 2nd page hotkeys
// Test with that for resolving hotkey conflicts

/// Available hotkey groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyGroupId {
    /// Hotkey commands when a unit is selected.
    UnitCommands = 0x0,
    /// Commands to control the game (speed, pause, UI toggles, etc).
    GameCommands = 0x1,
    /// Map scrolling commands.
    Scroll = 0x2,
    /// Villager build commands.
    Villager = 0x3,
    /// Commands when a Town Center is selected.
    TownCenter = 0x4,
    /// Commands when a Dock is selected.
    Dock = 0x5,
    /// Commands when a Barracks is selected.
    Barracks = 0x6,
    /// Commands when an Archery Range is selected.
    ArcheryRange = 0x7,
    /// Commands when a Stable is selected.
    Stable = 0x8,
    /// Commands when a Siege Workshop is selected.
    SiegeWorkshop = 0x9,
    /// Commands when a Monastery is selected.
    Monastery = 0xA,
    /// Commands when a Market is selected.
    Market = 0xB,
    /// Commands when a military unit is selected.
    MilitaryUnits = 0xC,
    /// Commands when a Castle is selected.
    Castle = 0xD,
    /// Commands when a Mill is selected.
    Mill = 0xE,
}

/// Hotkeys for castles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastleHotkeys {
    /// Create a Trebuchet.
    Trebuchet = 0x0,
    /// Create the unique unit.
    UniqueUnit = 0x1,
    /// Create a Petard.
    Petard = 0x2,
}

/// Hotkeys for military units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilitaryUnitHotkeys {
    /// Formation command.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    Formation = 0x0,
    /// Wheel the formation left. The `AoC`, HD Edition and `WololoKingdoms` files hold the
    /// Box formation in this slot (see [`FormationHotkeys`]).
    WheelLeft = 0x1,
    /// Wheel the formation right. The `AoC`, HD Edition and `WololoKingdoms` files hold
    /// the Line formation in this slot (see [`FormationHotkeys`]).
    WheelRight = 0x2,
    /// Turn the formation around. The `AoC`, HD Edition and `WololoKingdoms` files hold
    /// the Staggered formation in this slot (see [`FormationHotkeys`]).
    AboutFace = 0x3,
    /// Break up the formation. The `AoC`, HD Edition and `WololoKingdoms` files hold
    /// the Flank formation in this slot (see [`FormationHotkeys`]).
    DisbandFormation = 0x4,
    /// Patrol.
    Patrol = 0x5,
    /// Guard a unit or building.
    Guard = 0x6,
    /// Follow a unit.
    Follow = 0x7,
    /// Set the aggressive stance.
    Aggressive = 0x8,
    /// Set the defensive stance.
    Defensive = 0x9,
    /// Set the stand ground stance.
    StandGround = 0xA,
    /// Set the no attack stance.
    NoAttack = 0xB,
    /// HD Edition only.
    AttackMove = 0xC,
}

/// Hotkeys to change formations.
///
/// They index the [`HotkeyGroupId::MilitaryUnits`] group, like [`MilitaryUnitHotkeys`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormationHotkeys {
    /// Horde formation.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    Horde = 0x0,
    /// Box formation.
    Box = 0x1,
    /// Line formation.
    Line = 0x2,
    /// Staggered formation.
    Staggered = 0x3,
    /// Flank formation.
    Flank = 0x4,
}

/// Hotkeys to control units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitCommandHotkeys {
    /// Open the economic buildings a villager can build.
    BuildEconomic = 0x0,
    /// Open the military buildings a villager can build.
    BuildMilitary = 0x1,
    /// Repair.
    Repair = 0x2,
    /// Group the selected units.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    Group = 0x3,
    /// Ungroup the selected units.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    Ungroup = 0x4,
    /// Stop.
    Stop = 0x5,
    /// Unload a transport or ungarrison.
    Unload = 0x6,
    /// Pack a Trebuchet.
    Pack = 0x7,
    /// Unpack a Trebuchet.
    Unpack = 0x8,
    /// Heal (Monks).
    Heal = 0x9,
    /// Convert (Monks).
    Convert = 0xA,
    /// Garrison.
    Garrison = 0xB,
    /// Delete the selected unit.
    Delete = 0xC,
    /// Set the gather point of a building.
    SetGatherPoint = 0xD,
    /// Attack the ground.
    AttackGround = 0xE,
}

/// Hotkeys for the game UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCommandHotkeys {
    /// Create control group 0 (HD Edition shows it as #10).
    CreateGroup0 = 0x0,
    /// Create control group 1.
    CreateGroup1 = 0x1,
    /// Create control group 2.
    CreateGroup2 = 0x2,
    /// Create control group 3.
    CreateGroup3 = 0x3,
    /// Create control group 4.
    CreateGroup4 = 0x4,
    /// Create control group 5.
    CreateGroup5 = 0x5,
    /// Create control group 6.
    CreateGroup6 = 0x6,
    /// Create control group 7.
    CreateGroup7 = 0x7,
    /// Create control group 8.
    CreateGroup8 = 0x8,
    /// Create control group 9.
    CreateGroup9 = 0x9,
    /// Select control group 0 (HD Edition shows it as #10).
    SelectGroup0 = 0xA,
    /// Select control group 1.
    SelectGroup1 = 0xB,
    /// Select control group 2.
    SelectGroup2 = 0xC,
    /// Select control group 3.
    SelectGroup3 = 0xD,
    /// Select control group 4.
    SelectGroup4 = 0xE,
    /// Select control group 5.
    SelectGroup5 = 0xF,
    /// Select control group 6.
    SelectGroup6 = 0x10,
    /// Select control group 7.
    SelectGroup7 = 0x11,
    /// Select control group 8.
    SelectGroup8 = 0x12,
    /// Select control group 9.
    SelectGroup9 = 0x13,
    /// Send a chat message.
    Chat = 0x14,
    /// Speed up the game.
    SpeedUp = 0x15,
    /// Slow down the game.
    SpeedDown = 0x16,
    /// Go to the next idle villager.
    NextIdleVillager = 0x17,
    /// Go to the next idle villager (second binding).
    NextIdleVillager2 = 0x18,
    /// Go to the last notification.
    CycleFocusArea = 0x19,
    /// Go to the last notification (second binding).
    CycleFocusArea2 = 0x1A,
    /// Go to the selected object.
    GoToSelected = 0x1B,
    /// Go to a Town Center.
    GoToTownCenter = 0x1C,
    /// Go to a Town Center (second binding).
    GoToTownCenter2 = 0x1D,
    /// Go to a Market.
    GoToMarket = 0x1E,
    /// Review chat messages backward.
    ScrollChatDown = 0x1F,
    /// Review chat messages forward.
    ScrollChatUp = 0x20,
    /// Display the statistics.
    Score = 0x21,
    /// Go to a Barracks.
    GoToBarracks = 0x22,
    /// Go to an Archery Range.
    GoToArcheryRange = 0x23,
    /// Go to a Stable.
    GoToStable = 0x24,
    /// Go to a Siege Workshop.
    GoToSiegeWorkshop = 0x25,
    /// Go to a Dock.
    GoToDock = 0x26,
    /// Go to a Monastery.
    GoToMonastery = 0x27,
    /// Go to a Blacksmith.
    GoToBlacksmith = 0x28,
    /// Go to a Mill.
    GoToMill = 0x29,
    /// Go to a University.
    GoToUniversity = 0x2A,
    /// Display the technology tree.
    TechTree = 0x2B,
    /// Display the achievements.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    Achievements = 0x2C,
    /// Display the game time.
    DisplayGameTime = 0x2D,
    /// Go to the next idle military unit.
    NextIdleMilitary = 0x2E,
    /// Go to the next idle military unit (second binding).
    NextIdleMilitary2 = 0x2F,
    /// Send a flare.
    Flare = 0x30,
    /// Go to a Castle.
    GoToCastle = 0x31,
    /// Go to a Mining Camp.
    GoToMiningCamp = 0x32,
    /// Go to a Lumber Camp.
    GoToSawMill = 0x33,
    /// Switch the mini-map to combat mode.
    MinimapCombat = 0x34,
    /// Switch the mini-map to economic (resource) mode.
    MinimapResource = 0x35,
    /// Switch the mini-map to normal mode.
    MinimapNormal = 0x36,
    /// Toggle the extended help.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    ExtendedHelp = 0x37,
    /// Advanced modes.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    AdvancedModes = 0x38,
    /// Open the diplomacy screen.
    Diplomacy = 0x39,
    /// Open the game menu.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files
    /// (F10 is reserved for it).
    Menu = 0x3A,
    /// Open the objectives.
    Objectives = 0x3B,
    /// Open the chat dialog.
    ChatDialog = 0x3C,
    /// Pause the game.
    PauseGame = 0x3D,
    /// Save the game.
    SaveGame = 0x3E,
    /// Toggle friend or foe colours. Not in the original `AoC` files.
    FriendFoeColors = 0x3F,
    /// Return to the previous view. Not in the original `AoC` files.
    PreviousView = 0x40,
    /// Save a chapter. Not in the original `AoC` files.
    SaveChapter = 0x41,
}

/// Hotkeys for scrolling.
///
/// Only the first four are bound in the `AoC`, HD Edition and `WololoKingdoms` files; the others
/// have string id -1 there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollHotkeys {
    /// Scroll left.
    ScrollLeft = 0x0,
    /// Scroll right.
    ScrollRight = 0x1,
    /// Scroll up.
    ScrollUp = 0x2,
    /// Scroll down.
    ScrollDown = 0x3,
    /// Scroll left fast.
    FastScrollLeft = 0x4,
    /// Scroll right fast.
    FastScrollRight = 0x5,
    /// Scroll up fast.
    FastScrollUp = 0x6,
    /// Scroll down fast.
    FastScrollDown = 0x7,
    /// Scroll left and up.
    ScrollLeftUp = 0x8,
    /// Scroll left and down.
    ScrollLeftDown = 0x9,
    /// Scroll right and down.
    ScrollRightDown = 0xA,
    /// Scroll right and up.
    ScrollRightUp = 0xB,
    /// Scroll left and up fast.
    FastScrollLeftUp = 0xC,
    /// Scroll left and down fast.
    FastScrollLeftDown = 0xD,
    /// Scroll right and down fast.
    FastScrollRightDown = 0xE,
    /// Scroll right and up fast.
    FastScrollRightUp = 0xF,
}

/// Hotkeys for villagers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VillagerHotkeys {
    /// Build a House.
    BuildHouse = 0x0,
    /// Build a Mill.
    BuildMill = 0x1,
    /// Build a Blacksmith.
    BuildBlacksmith = 0x2,
    /// Build a Dock.
    BuildDock = 0x3,
    /// Build a Barracks.
    BuildBarracks = 0x4,
    /// Build a Palisade Wall.
    BuildPalisadeWall = 0x5,
    /// Build a Market.
    BuildMarket = 0x6,
    /// Build a Stone Wall.
    BuildStoneWall = 0x7,
    /// Build a tower.
    BuildTower = 0x8,
    /// Build a Bombard Tower.
    BuildBombardTower = 0x9,
    /// Build a Gate.
    BuildGate1 = 0xA,
    /// Build a Gate (second slot).
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    BuildGate2 = 0xB,
    /// Build a Farm.
    BuildFarm = 0xC,
    /// Build an Archery Range.
    BuildArcheryRange = 0xD,
    /// Build a Stable.
    BuildStable = 0xE,
    /// Build a Monastery.
    BuildMonastery = 0xF,
    /// Build a Town Center.
    BuildTownCenter = 0x10,
    /// Build a Siege Workshop.
    BuildSiegeWorkshop = 0x11,
    /// Build a University.
    BuildUniversity = 0x12,
    /// Build a Wonder.
    BuildWonder = 0x13,
    /// Build a Castle.
    BuildCastle = 0x14,
    /// Build a Trade Workshop.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    BuildTradeWorkshop = 0x15,
    /// Build a packed Town Center.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    BuildPackedTownCenter = 0x16,
    /// Build a Lumber Camp.
    BuildSawMill = 0x17,
    /// Build a Mining Camp.
    BuildMiningCamp = 0x18,
    /// Build a Fish Trap.
    BuildFishTrap = 0x19,
    /// Build an Outpost.
    BuildOutpost = 0x1A,
    /// Show the next page of buildings.
    BuildNext = 0x1B,
    /// Build a Palisade Gate. Only in the `WololoKingdoms` file.
    BuildPalisadeGate = 0x1C,
    /// Build a Feitoria. Only in the `WololoKingdoms` file.
    BuildFeitoria = 0x1D,
}

/// Hotkeys for the town center.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TownCenterHotkeys {
    /// Train a villager.
    CreateVillager = 0x0,
    /// Advance to the next age.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    AgeAdvance = 0x1,
    /// Named for a raider archer.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    CreateRaiderArcher = 0x2,
    /// Named for a raider swordsman.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    CreateRaiderSwordsman = 0x3,
    /// Named for a raider cavalry unit.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    CreateRaiderCavalry = 0x4,
    /// Named for a raider cavalry archer.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    CreateRaiderCavArcher = 0x5,
    /// Ring the town bell.
    RingTownBell = 0x6,
    /// Send the garrisoned villagers back to work.
    BackToWork = 0x7,
}

/// Hotkeys for mills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MillHotkeys {
    /// Queue a farm reseed.
    AddFarm = 0x0,
}

/// Hotkeys for docks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockHotkeys {
    /// Build a Fishing Ship.
    CreateFishingShip = 0x0,
    /// Build a Trade Cog.
    CreateTradeCog = 0x1,
    /// Build a Galley (or its upgrade).
    CreateGalley = 0x2,
    /// Build a Cannon Galleon.
    CreateCannonGalleon = 0x3,
    /// Build a Fire Ship.
    CreateFireGalley = 0x4,
    /// Build a Demolition Ship.
    CreateDemolitionShip = 0x5,
    /// Build a boarding galley.
    /// Unused (string id -1) in the `AoC`, HD Edition and `WololoKingdoms` files.
    CreateBoardingGalley = 0x6,
    /// Build a Transport Ship.
    CreateTransport = 0x7,
    /// Build a Longboat.
    CreateLongboat = 0x8,
    /// Build a Turtle Ship. Not in the original `AoC` files.
    CreateTurtleShip = 0x9,
}

/// Hotkeys for the barracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarracksHotkeys {
    /// Train a Militia (or its upgrade).
    CreateMilitia = 0x0,
    /// Train a Spearman (or its upgrade).
    CreatePikeman = 0x1,
    /// Train an Eagle Warrior. Not in the original `AoC` files.
    CreateEagleWarrior = 0x2,
    /// Train a Huskarl. Not in the original `AoC` files.
    CreateHuskarl = 0x3,
}

/// Hotkeys for archery ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcheryRangeHotkeys {
    /// Train an Archer (or its upgrade).
    CreateArcher = 0x0,
    /// Train a Skirmisher.
    CreateSkirmisher = 0x1,
    /// Train a Cavalry Archer.
    CreateCavArcher = 0x2,
    /// Train a Hand Cannoneer.
    CreateHandCannoneer = 0x3,
    /// Train a Genitour. Only in the `WololoKingdoms` file.
    CreateGenitour = 0x4,
}

/// Hotkeys for stables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableHotkeys {
    /// Train a Scout Cavalry (or its upgrade).
    CreateScout = 0x0,
    /// Train a Camel.
    CreateCamel = 0x1,
    /// Train a Knight (or its upgrade).
    CreateKnight = 0x2,
    /// Train a Battle Elephant. Not in the `AoC`, HD Edition and
    /// `WololoKingdoms` files.
    CreateBattleElephant = 0x3,
}

/// Hotkeys for siege workshops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiegeWorkshopHotkeys {
    /// Build a Battering Ram.
    CreateRam = 0x0,
    /// Build a Scorpion.
    CreateScorpion = 0x1,
    /// Build a Mangonel.
    CreateMangonel = 0x2,
    /// Build a Bombard Cannon.
    CreateBombardCannon = 0x3,
    /// Build a Siege Tower. Only in the `WololoKingdoms` file.
    CreateSiegeTower = 0x4,
}

/// Hotkeys for the monastery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonasteryHotkeys {
    /// Train a Monk.
    CreateMonk = 0x0,
    /// Train a Missionary. Not in the original `AoC` files.
    CreateMissionary = 0x1,
}

/// Hotkeys for the market.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketHotkeys {
    /// Build a Trade Cart.
    CreateTradeCart = 0x0,
}

/// Hotkeys for the blacksmith.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlacksmithHotkeys {}

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
    index: usize,

    /// The number of groups, must be less than or equal to `index`.
    num_groups: usize,
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
    index: usize,

    /// The number of hotkeys, must be less than or equal to `index`.
    num_hotkeys: usize,
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

/// The information about a single hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    /// Keycode that activates this hotkey.
    ///
    /// You can use a crate like [keycodes](https://docs.rs/keycodes/0.1.0/)
    /// to compare this to named virtual keys like `keycodes::KEY_RETURN`.
    ///
    /// If `string_id` is `-1`, this `key` must be `0`.
    pub key: i32,

    /// The string ID for this hotkey's label. -1 if this hotkey is unused.
    pub string_id: i32,

    /// Whether the Ctrl key needs to be held to activate this hotkey.
    pub ctrl: bool,

    /// Whether the Alt key needs to be held to activate this hotkey.
    pub alt: bool,

    /// Whether the Shift key needs to be held to activate this hotkey.
    pub shift: bool,

    /// Not sure what this does yet? Actually may be unused…
    mouse: i8,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            key: 0,
            string_id: -1,
            ctrl: false,
            alt: false,
            shift: false,
            mouse: 0,
        }
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {}{}{}{}",
            self.string_id,
            if self.ctrl { "Ctrl-" } else { "" },
            if self.alt { "Alt-" } else { "" },
            if self.shift { "Shift-" } else { "" },
            self.key
        )
    }
}

impl Hotkey {
    /// Returns a hotkey equivalent to this one but with the key set to `key`.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::Hotkey;
    ///
    /// let hki = Hotkey::default();
    /// let hki2 = hki.key(5);
    /// assert_eq!(0, hki.key);
    /// assert_eq!(5, hki2.key);
    /// ```
    #[must_use]
    pub const fn key(self, key: i32) -> Self {
        Self { key, ..self }
    }

    /// Returns a hotkey equivalent to this one but with the string key set
    /// to `string_id`.
    /// If the id is `-1`, the returned key also has a `key` field set to `0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::Hotkey;
    ///
    /// let hki = Hotkey::default();
    /// let hki2 = hki.string_id(5);
    /// assert_eq!(-1, hki.string_id);
    /// assert_eq!(5, hki2.string_id);
    ///
    /// let hki3 = hki2.key(5);
    /// let hki4 = hki3.string_id(-1);
    /// assert_eq!(5, hki3.key);
    /// assert_eq!(0, hki4.key);
    /// ```
    #[must_use]
    pub const fn string_id(self, string_id: i32) -> Self {
        if string_id == -1 {
            Self {
                string_id,
                key: 0,
                ..self
            }
        } else {
            Self { string_id, ..self }
        }
    }

    /// Returns a hotkey equivalent to this one but with `ctrl` determining
    /// whether control must be held when pressing the key.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::Hotkey;
    ///
    /// let hki = Hotkey::default();
    /// let hki2 = hki.ctrl(true);
    /// assert!(!hki.ctrl);
    /// assert!(hki2.ctrl);
    /// ```
    #[must_use]
    pub const fn ctrl(self, ctrl: bool) -> Self {
        Self { ctrl, ..self }
    }

    /// Returns a hotkey equivalent to this one but with `alt` determining
    /// whether alt must be held when pressing the key.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::Hotkey;
    ///
    /// let hki = Hotkey::default();
    /// let hki2 = hki.alt(true);
    /// assert!(!hki.alt);
    /// assert!(hki2.alt);
    /// ```
    #[must_use]
    pub const fn alt(self, alt: bool) -> Self {
        Self { alt, ..self }
    }

    /// Returns a hotkey equivalent to this one but with `shift` determining
    /// whether shift must be held when pressing the key.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::Hotkey;
    ///
    /// let hki = Hotkey::default();
    /// let hki2 = hki.shift(true);
    /// assert!(!hki.shift);
    /// assert!(hki2.shift);
    /// ```
    #[must_use]
    pub const fn shift(self, shift: bool) -> Self {
        Self { shift, ..self }
    }

    /// Reads a hotkey from an input stream.
    pub(crate) fn from<R: Read>(input: &mut R) -> io::Result<Self> {
        let key = input.read_i32::<LE>()?;
        let string_id = input.read_i32::<LE>()?;
        let ctrl = input.read_u8()? != 0;
        let alt = input.read_u8()? != 0;
        let shift = input.read_u8()? != 0;
        let mouse = input.read_i8()?;

        Ok(Self {
            key,
            string_id,
            ctrl,
            alt,
            shift,
            mouse,
        })
    }

    /// Writes a hotkey to an output stream.
    pub(crate) fn write_to<W: Write>(&self, output: &mut W) -> io::Result<()> {
        output.write_i32::<LE>(self.key)?;
        output.write_i32::<LE>(self.string_id)?;
        output.write_u8(u8::from(self.ctrl))?;
        output.write_u8(u8::from(self.alt))?;
        output.write_u8(u8::from(self.shift))?;
        output.write_i8(self.mouse)?;
        Ok(())
    }

    /// Returns a string representation of this hotkey, using the strings from
    /// `lang_file`.
    ///
    /// # Examples
    ///
    /// ```
    /// use genie_hki::Hotkey;
    /// use genie_lang::{LangFile, StringKey};
    ///
    /// let mut lang_file = LangFile::new();
    /// lang_file.insert(StringKey::from(5u32), String::from("A"));
    /// let hotkey = Hotkey::default().key(65).string_id(5).ctrl(true);
    /// assert_eq!("A (5): ctrl-65", hotkey.get_string_from_lang(&lang_file));
    ///
    /// let default = Hotkey::default();
    /// assert_eq!("-1: 0", default.get_string_from_lang(&lang_file));
    /// ```
    #[must_use]
    pub fn get_string_from_lang(&self, lang_file: &LangFile) -> String {
        let ctrl = if self.ctrl { "ctrl-" } else { "" };
        let alt = if self.alt { "ctrl-" } else { "" };
        let shift = if self.shift { "ctrl-" } else { "" };

        // Only positive ids name a string.
        let name = u32::try_from(self.string_id)
            .ok()
            .filter(|&id| id > 0)
            .and_then(|id| lang_file.get(&StringKey::from(id)));

        name.map_or_else(
            || format!("{}: {}{}{}{}", self.string_id, ctrl, alt, shift, self.key),
            |s| {
                format!(
                    "{} ({}): {}{}{}{}",
                    s, self.string_id, ctrl, alt, shift, self.key
                )
            },
        )
    }
}

/// Represents a group of `Hotkey`s.
///
/// Different groups may have different numbers of hotkeys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyGroup {
    /// The hotkeys in this group, ordered by the order they appear in a
    /// hotkey file.
    hotkeys: Vec<Hotkey>,
}

impl HotkeyGroup {
    /// Reads a hotkey group from an input stream.
    pub(crate) fn from<R: Read>(input: &mut R) -> io::Result<Self> {
        let num_hotkeys = input.read_u32::<LE>()?;
        let mut hotkeys = Vec::with_capacity(num_hotkeys as usize);
        for _ in 0..num_hotkeys {
            hotkeys.push(Hotkey::from(input)?);
        }

        Ok(Self { hotkeys })
    }

    /// Writes a hotkey group to an output stream.
    // The format stores the hotkey count as a u32.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn write_to<W: Write>(&self, output: &mut W) -> io::Result<()> {
        output.write_u32::<LE>(self.hotkeys.len() as u32)?;
        for hotkey in &self.hotkeys {
            hotkey.write_to(output)?;
        }
        Ok(())
    }

    /// Returns an immutable reference to a single hotkey, if that hotkey is
    /// present in this `HotkeyGroup`.
    #[must_use]
    pub fn hotkey(&self, index: usize) -> Option<&Hotkey> {
        self.hotkeys.get(index)
    }

    /// Get a mutable reference to a single hotkey.
    /// This way, you can edit or replace the mapping.
    pub fn hotkey_mut(&mut self, index: usize) -> Option<&mut Hotkey> {
        self.hotkeys.get_mut(index)
    }

    /// Returns a hotkey group equivalent to this group but with the hotkey
    /// at `index` unbound so that the key is `0` and all modifier keys are
    /// `false`.
    ///
    /// # Errors
    ///
    /// Returns a `HotkeyIndexError` if the group has no hotkey at `index`.
    pub fn unbind(&self, index: usize) -> Result<Self, HotkeyIndexError> {
        self.bind(index, 0, false, false, false)
    }

    /// Returns a hotkey group equivalent to this group but with the hotkey
    /// at `index` bound with the given key and modifier keys.
    ///
    /// # Errors
    ///
    /// Returns a `HotkeyIndexError` if the group has no hotkey at `index`.
    pub fn bind(
        &self,
        index: usize,
        key: i32,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Result<Self, HotkeyIndexError> {
        if index >= self.num_hotkeys() {
            return Err(HotkeyIndexError::new(index, self.num_hotkeys()));
        }
        let hotkeys = self
            .hotkeys
            .iter()
            .enumerate()
            .map(|(i, &hk)| {
                if i == index {
                    hk.key(key).ctrl(ctrl).alt(alt).shift(shift)
                } else {
                    hk
                }
            })
            .collect();
        Ok(Self { hotkeys })
    }

    /// Returns the number of hotkeys in this `HotkeyGroup`.
    /// ```rust
    /// # fn main() -> anyhow::Result<()> {
    /// use std::fs::File;
    /// use genie_hki::HotkeyInfo;
    /// let mut f = File::open("test/files/aoc1.hki")?;
    /// let info = HotkeyInfo::from(&mut f)?;
    /// // Villager hotkeys are group 3 in AoC and HD Edition.
    /// let group = info.group(3).expect("file does not contain villager hotkeys");
    /// assert_eq!(28, group.num_hotkeys());
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub const fn num_hotkeys(&self) -> usize {
        self.hotkeys.len()
    }

    /// Returns an iterator over this group's hotkeys.
    pub fn iter(&self) -> impl Iterator<Item = &Hotkey> {
        self.hotkeys.iter()
    }

    /// Returns an iterator over mutable references to this group's hotkeys.
    pub fn iter_mut(&mut self) -> slice::IterMut<'_, Hotkey> {
        self.hotkeys.iter_mut()
    }

    /// Returns a string representation of this hotkey group, using the strings
    /// from `lang_file` and the group name string key `sk`.
    #[must_use]
    pub fn get_string_from_lang(&self, lang_file: &LangFile, sk: &StringKey) -> String {
        let group_name = lang_file
            .get(sk)
            .map_or_else(String::new, |name| format!("{name} ({sk}):\n  "));
        let hotkeys: Vec<String> = self
            .hotkeys
            .iter()
            .map(|hki| hki.get_string_from_lang(lang_file))
            .collect();
        format!("{}{}", group_name, hotkeys.join("\n  "))
    }
}

impl fmt::Display for HotkeyGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let group_string = if self.hotkeys.is_empty() {
            String::from(" no hotkeys")
        } else {
            let hotkeys: Vec<String> = self.hotkeys.iter().map(ToString::to_string).collect();
            format!("\n  {}", hotkeys.join("\n  "))
        };
        write!(f, "Group:{group_string}")
    }
}

impl IntoIterator for HotkeyGroup {
    type Item = Hotkey;
    type IntoIter = vec::IntoIter<Hotkey>;
    fn into_iter(self) -> Self::IntoIter {
        self.hotkeys.into_iter()
    }
}

impl<'a> IntoIterator for &'a HotkeyGroup {
    type Item = &'a Hotkey;
    type IntoIter = slice::Iter<'a, Hotkey>;
    fn into_iter(self) -> Self::IntoIter {
        self.hotkeys.iter()
    }
}

impl<'a> IntoIterator for &'a mut HotkeyGroup {
    type Item = &'a mut Hotkey;
    type IntoIter = slice::IterMut<'a, Hotkey>;
    fn into_iter(self) -> Self::IntoIter {
        self.hotkeys.iter_mut()
    }
}

/// Represents a HKI file containing hotkey settings.
#[derive(Debug, Clone, PartialEq)]
pub struct HotkeyInfo {
    /// The file version.
    version: f32,

    /// The hotkey groups.
    groups: Vec<HotkeyGroup>,
}

impl HotkeyInfo {
    /// Read a hotkey info structure from an uncompressed stream.
    fn from_uncompressed<R: Read>(input: &mut R) -> io::Result<Self> {
        let version = input.read_f32::<LE>()?;
        let num_groups = input.read_u32::<LE>()?;
        let mut groups = Vec::with_capacity(num_groups as usize);
        for _ in 0..num_groups {
            groups.push(HotkeyGroup::from(input)?);
        }

        Ok(Self { version, groups })
    }

    /// Reads a hotkey info file.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from `input` fails, including when the data is not valid
    /// DEFLATE or ends early.
    pub fn from<R: Read>(input: &mut R) -> io::Result<Self> {
        let mut input = DeflateDecoder::new(input);
        Self::from_uncompressed(&mut input)
    }

    /// Writes a hotkey info structure to an uncompressed stream.
    // The format stores the group count as a u32.
    #[allow(clippy::cast_possible_truncation)]
    fn write_to_uncompressed<W: Write>(&self, output: &mut W) -> io::Result<()> {
        output.write_f32::<LE>(self.version)?;
        output.write_u32::<LE>(self.groups.len() as u32)?;
        for group in &self.groups {
            group.write_to(output)?;
        }
        Ok(())
    }

    /// Writes a hotkey info file.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to `output` fails.
    pub fn write_to<W: Write>(&self, output: &mut W) -> io::Result<()> {
        let mut output = DeflateEncoder::new(output, Compression::default());
        self.write_to_uncompressed(&mut output)
    }

    /// Gets the file version.
    ///
    /// ```rust
    /// # fn main() -> anyhow::Result<()> {
    /// use std::fs::File;
    /// use genie_hki::HotkeyInfo;
    /// let mut f = File::open("test/files/aoc1.hki")?;
    /// let info = HotkeyInfo::from(&mut f).expect("failed to read file");
    /// assert_eq!(info.version(), 1.0);
    ///
    /// let mut f = File::open("test/files/hd0.hki")?;
    /// let info = HotkeyInfo::from(&mut f).expect("failed to read file");
    /// assert_eq!(info.version(), 3.0);
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub const fn version(&self) -> f32 {
        self.version
    }

    /// Returns an immutable reference to a hotkey group, if that group exists.
    ///
    /// ```rust
    /// # fn main() -> anyhow::Result<()> {
    /// use std::fs::File;
    /// use genie_hki::{HotkeyInfo, HotkeyGroupId};
    /// let mut f = File::open("test/files/aoc1.hki")?;
    /// let info = HotkeyInfo::from(&mut f).expect("failed to read file");
    /// assert!(info.group(HotkeyGroupId::Villager as usize).is_some());
    /// assert!(info.group(HotkeyGroupId::Mill as usize).is_none());
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub fn group(&self, group_id: usize) -> Option<&HotkeyGroup> {
        self.groups.get(group_id)
    }

    /// Returns a mutable reference to a hotkey group, if that group exists.
    pub fn group_mut(&mut self, group_id: usize) -> Option<&mut HotkeyGroup> {
        self.groups.get_mut(group_id)
    }

    /// Returns the number of hotkey groups in this info's hotkey file.
    /// ```rust
    /// # fn main() -> anyhow::Result<()> {
    /// use std::fs::File;
    /// use genie_hki::HotkeyInfo;
    ///
    /// let mut f = File::open("test/files/aoc1.hki")?;
    /// let info = HotkeyInfo::from(&mut f).expect("failed to read file");
    /// assert_eq!(14, info.num_groups());
    ///
    /// let mut f = File::open("test/files/wk.hki")?;
    /// let info = HotkeyInfo::from(&mut f).expect("failed to read file");
    /// assert_eq!(15, info.num_groups());
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub const fn num_groups(&self) -> usize {
        self.groups.len()
    }

    /// Returns an iterator over the hotkey groups present in this info's hotkey
    /// file.
    pub fn iter(&self) -> impl Iterator<Item = &HotkeyGroup> {
        self.groups.iter()
    }

    /// Returns an iterator over mutable references to the hotkey groups of this info's hotkey
    /// file.
    pub fn iter_mut(&mut self) -> slice::IterMut<'_, HotkeyGroup> {
        self.groups.iter_mut()
    }

    /// Returns a `HotkeyInfo` struct equivalent to this `HotkeyInfo`, but with
    /// the key at index `key_index` of the group given by `group_index`
    /// unbound.
    ///
    /// # Errors
    ///
    /// Returns an `IndexError` if there is no group at `group_index` or no hotkey at
    /// `key_index` in it.
    pub fn unbind_key(&self, group_index: usize, key_index: usize) -> Result<Self, IndexError> {
        self.bind_key(group_index, key_index, 0, false, false, false)
    }

    /// Returns a `HotkeyInfo` struct equivalent to this `HotkeyInfo`, but with
    /// the key at index `key_index` of the group given by `group_index`
    /// bound with the given key and key modifiers.
    ///
    /// # Errors
    ///
    /// Returns an `IndexError` if there is no group at `group_index` or no hotkey at
    /// `key_index` in it.
    pub fn bind_key(
        &self,
        group_index: usize,
        key_index: usize,
        key: i32,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Result<Self, IndexError> {
        if group_index >= self.num_groups() {
            return Err(IndexError::GroupIndex(GroupIndexError::new(
                group_index,
                self.num_groups(),
            )));
        }
        let mut groups = Vec::with_capacity(self.num_groups());
        for (i, grp) in self.groups.iter().enumerate() {
            let append = if i == group_index {
                grp.bind(key_index, key, ctrl, alt, shift)?
            } else {
                grp.clone()
            };
            groups.push(append);
        }
        Ok(Self { groups, ..*self })
    }

    /// Returns a map `keycode -> vec[hotkey1, hotkey2, ... hotkeyn]` mapping
    /// every used keybinding to all of the hotkeys to which it is assigned.
    ///
    /// Note hotkeys may have different behavior in different contexts, such
    /// as `A` producing an archer when an Archery Range is selected and
    /// a militia when a Barracks is selected.
    pub fn bindings_per_keycode(&self) -> HashMap<i32, Vec<Hotkey>> {
        let mut bindings = HashMap::new();
        for group in self.iter() {
            for hotkey in group.iter() {
                bindings
                    .entry(hotkey.key)
                    .or_insert_with(Vec::new)
                    .push(*hotkey);
            }
        }
        bindings
    }

    /// Returns a string representation of this `HotkeyInfo` struct using the
    /// strings from `lang_file` and the group name string keys given in `him`.
    ///
    /// # Panics
    ///
    /// Panics if the number of hotkeys in any group of this file differs from
    /// the number of hotkeys given to that group in `him`.
    #[must_use]
    pub fn get_string_from_lang(&self, lang_file: &LangFile, him: &HotkeyInfoMetadata) -> String {
        let groups: Vec<String> = self
            .groups
            .iter()
            .zip(him.iter())
            .map(|(grp, sk)| grp.get_string_from_lang(lang_file, sk))
            .collect();
        format!("Version: {}\n{}", self.version, groups.join("\n"))
    }
}

impl fmt::Display for HotkeyInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let group_string = if self.groups.is_empty() {
            String::new()
        } else {
            let groups: Vec<String> = self.groups.iter().map(ToString::to_string).collect();
            format!("\n{}", groups.join("\n"))
        };
        write!(f, "Version: {}{}", self.version, group_string)
    }
}

impl IntoIterator for HotkeyInfo {
    type Item = HotkeyGroup;
    type IntoIter = vec::IntoIter<HotkeyGroup>;
    fn into_iter(self) -> Self::IntoIter {
        self.groups.into_iter()
    }
}

impl<'a> IntoIterator for &'a HotkeyInfo {
    type Item = &'a HotkeyGroup;
    type IntoIter = slice::Iter<'a, HotkeyGroup>;
    fn into_iter(self) -> Self::IntoIter {
        self.groups.iter()
    }
}

impl<'a> IntoIterator for &'a mut HotkeyInfo {
    type Item = &'a mut HotkeyGroup;
    type IntoIter = slice::IterMut<'a, HotkeyGroup>;
    fn into_iter(self) -> Self::IntoIter {
        self.groups.iter_mut()
    }
}

impl From<IndexError> for genie_error::GenieError {
    /// The error as a [`genie_error::GenieError`]: its kind and reason, the original as the
    /// source.
    #[track_caller]
    fn from(e: IndexError) -> Self {
        let to = match &e {
            IndexError::GroupIndex(_) => {
                Self::invalid_input("a hotkey group index past the groups")
            }
            IndexError::HotkeyIndex(_) => {
                Self::invalid_input("a hotkey index past the group's hotkeys")
            }
        };
        to.with_source(e)
    }
}

impl From<GroupIndexError> for genie_error::GenieError {
    /// The error as a [`genie_error::GenieError`]: its kind and reason, the original as the
    /// source.
    #[track_caller]
    fn from(e: GroupIndexError) -> Self {
        Self::invalid_input("a hotkey group index past the groups").with_source(e)
    }
}

impl From<HotkeyIndexError> for genie_error::GenieError {
    /// The error as a [`genie_error::GenieError`]: its kind and reason, the original as the
    /// source.
    #[track_caller]
    fn from(e: HotkeyIndexError) -> Self {
        Self::invalid_input("a hotkey index past the group's hotkeys").with_source(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use std::fs::File;

    #[test]
    fn aoc1() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        // The test only checks that the file reads.
        let _ = HotkeyInfo::from(&mut f).context("failed to read file")?;
        Ok(())
    }

    #[test]
    fn aoc2() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc2.hki")?;
        // The test only checks that the file reads.
        let _ = HotkeyInfo::from(&mut f).context("failed to read file")?;
        Ok(())
    }

    #[test]
    fn aoc3() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc3.hki")?;
        // The test only checks that the file reads.
        let _ = HotkeyInfo::from(&mut f).context("failed to read file")?;
        Ok(())
    }

    #[test]
    fn hd0() -> anyhow::Result<()> {
        let mut f = File::open("test/files/hd0.hki")?;
        // The test only checks that the file reads.
        let _ = HotkeyInfo::from(&mut f).context("failed to read file")?;
        Ok(())
    }

    #[test]
    fn hd1() -> anyhow::Result<()> {
        let mut f = File::open("test/files/hd1.hki")?;
        // The test only checks that the file reads.
        let _ = HotkeyInfo::from(&mut f).context("failed to read file")?;
        Ok(())
    }

    #[test]
    fn wk() -> anyhow::Result<()> {
        let mut f = File::open("test/files/wk.hki")?;
        // The test only checks that the file reads.
        let _ = HotkeyInfo::from(&mut f).context("failed to read file")?;
        Ok(())
    }

    #[test]
    fn hk_group_iter() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let group = info.group(HotkeyGroupId::UnitCommands as usize).unwrap();
        let mut hotkey_iter = group.iter();
        assert_eq!(19214, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19215, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19224, hotkey_iter.next().unwrap().string_id);
        assert_eq!(-1, hotkey_iter.next().unwrap().string_id);
        assert_eq!(-1, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19216, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19225, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19241, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19242, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19221, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19222, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19012, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19000, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19002, hotkey_iter.next().unwrap().string_id);
        assert_eq!(19220, hotkey_iter.next().unwrap().string_id);
        assert_eq!(None, hotkey_iter.next());
        Ok(())
    }

    #[test]
    fn hk_info_iter() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let mut iter = info.iter();
        assert_eq!(
            info.group(HotkeyGroupId::UnitCommands as usize),
            iter.next()
        );
        assert_eq!(
            info.group(HotkeyGroupId::GameCommands as usize),
            iter.next()
        );
        assert_eq!(info.group(HotkeyGroupId::Scroll as usize), iter.next());
        assert_eq!(info.group(HotkeyGroupId::Villager as usize), iter.next());
        assert_eq!(info.group(HotkeyGroupId::TownCenter as usize), iter.next());
        assert_eq!(info.group(HotkeyGroupId::Dock as usize), iter.next());
        assert_eq!(info.group(HotkeyGroupId::Barracks as usize), iter.next());
        assert_eq!(
            info.group(HotkeyGroupId::ArcheryRange as usize),
            iter.next()
        );
        assert_eq!(info.group(HotkeyGroupId::Stable as usize), iter.next());
        assert_eq!(
            info.group(HotkeyGroupId::SiegeWorkshop as usize),
            iter.next()
        );
        assert_eq!(info.group(HotkeyGroupId::Monastery as usize), iter.next());
        assert_eq!(info.group(HotkeyGroupId::Market as usize), iter.next());
        assert_eq!(
            info.group(HotkeyGroupId::MilitaryUnits as usize),
            iter.next()
        );
        assert_eq!(info.group(HotkeyGroupId::Castle as usize), iter.next());
        assert_eq!(None, iter.next());
        Ok(())
    }

    #[test]
    fn hk_group_unbind() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let group0 = info.group(HotkeyGroupId::UnitCommands as usize).unwrap();
        let group1 = group0.unbind(UnitCommandHotkeys::BuildEconomic as usize)?;
        assert_eq!(66, group0.hotkey(0).unwrap().key);
        assert_eq!(0, group1.hotkey(0).unwrap().key);
        Ok(())
    }

    #[test]
    fn hk_group_bind() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let group0 = info.group(HotkeyGroupId::UnitCommands as usize).unwrap();
        let group1 = group0.bind(
            UnitCommandHotkeys::BuildEconomic as usize,
            65,
            false,
            false,
            false,
        )?;
        assert_eq!(66, group0.hotkey(0).unwrap().key);
        assert_eq!(65, group1.hotkey(0).unwrap().key);
        Ok(())
    }

    #[test]
    fn hk_group_bad_index() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let group = info.group(HotkeyGroupId::UnitCommands as usize).unwrap();
        let result = group.unbind(99999);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn hk_info_unbind() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info0 = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let info1 = info0.unbind_key(0, 0)?;
        assert_eq!(
            66,
            info0
                .group(HotkeyGroupId::UnitCommands as usize)
                .unwrap()
                .hotkey(0)
                .unwrap()
                .key
        );
        assert_eq!(
            0,
            info1
                .group(HotkeyGroupId::UnitCommands as usize)
                .unwrap()
                .hotkey(0)
                .unwrap()
                .key
        );
        Ok(())
    }

    #[test]
    fn hk_info_bind() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info0 = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let info1 = info0.bind_key(0, 0, 65, false, false, false)?;
        assert_eq!(
            66,
            info0
                .group(HotkeyGroupId::UnitCommands as usize)
                .unwrap()
                .hotkey(0)
                .unwrap()
                .key
        );
        assert_eq!(
            65,
            info1
                .group(HotkeyGroupId::UnitCommands as usize)
                .unwrap()
                .hotkey(0)
                .unwrap()
                .key
        );
        Ok(())
    }

    #[test]
    fn hk_info_bad_index_group() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let result = info.unbind_key(999_999, 0);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn hk_info_bad_index_hk() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let result = info.unbind_key(0, 999_999);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_keycode_to_bindings_map() -> anyhow::Result<()> {
        let mut f = File::open("test/files/aoc1.hki")?;
        let info = HotkeyInfo::from(&mut f).context("failed to read file")?;
        let map = info.bindings_per_keycode();
        // 19270: Ctrl-65
        let h0 = Hotkey::default().string_id(19270).key(65).ctrl(true);
        // 19062: 65
        let h1 = Hotkey::default().string_id(19062).key(65);
        // 19059: 65
        let h2 = Hotkey::default().string_id(19059).key(65);
        // 19038: 65
        let h3 = Hotkey::default().string_id(19038).key(65);
        // 19285: 65
        let h4 = Hotkey::default().string_id(19285).key(65);
        // 19315: 65
        let h5 = Hotkey::default().string_id(19315).key(65);
        let hotkeys = vec![h0, h1, h2, h3, h4, h5];
        assert_eq!(Some(&hotkeys), map.get(&65));
        Ok(())
    }
}

#[cfg(test)]
mod genie_error_tests {
    use super::*;

    /// The errors convert into `GenieError`s with their kinds and reasons, the original as
    /// source.
    #[test]
    fn index_errors_convert_into_genie_errors() {
        let e: genie_error::GenieError = GroupIndexError::new(5, 3).into();
        assert_eq!(
            (e.kind(), e.reason()),
            (
                genie_error::ErrorKind::InvalidInput,
                "a hotkey group index past the groups"
            )
        );
        let e: genie_error::GenieError =
            IndexError::HotkeyIndex(HotkeyIndexError::new(9, 2)).into();
        assert_eq!(e.reason(), "a hotkey index past the group's hotkeys");
        assert!(std::error::Error::source(&e).is_some());
    }
}
