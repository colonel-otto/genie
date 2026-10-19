//! The fog's ghosts (`RGE_Doppleganger_Object`, type 25, each player's master 243 "DOPL"): what a
//! player keeps seeing of an object it lost sight of. An object whose master leaves ghosts
//! (`create_doppelganger_on_death` 2, a mine or a tree say) leaves one at its death for every
//! player who explored its tile but does not see it (`FUN_14118e990`), and its owner's watched
//! list (`player+0x78`) then follows the tile: a player who loses sight of it gets one too
//! (`FUN_1410905e0`, after the player loop). A building in its owner's list is followed by its own
//! visibility mask (`+0x8f8`). A ghost goes once its viewer sees its tile again (`FUN_141090000`,
//! the destroy slot 0xd0), two updates later it leaves for the world's pool of type 25, and the
//! next ghost takes it back; a ghost takes the world's next reusable id otherwise, which the
//! recycled missiles share.
//!
//! 158041: the list's add `FUN_141090430` and removal `FUN_141091310`; a ghost's making
//! `FUN_1410cfe80` (constructor `FUN_14108dae0`, the pool's `FUN_1410909f0`), its link to the
//! original `FUN_14108f9b0`, the twin check `FUN_1410928a0`, its update `FUN_141090000`, its
//! destroy `FUN_14118f1e0` and its leaving `FUN_141090ad0`.

use crate::registry::MASK;
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// The ghosts' master in every player's list (`player+0x28`'s entry 243, "DOPL").
pub const GHOST_MASTER: i16 = 243;
/// The type of a ghost (`+0xf`).
pub const GHOST: u8 = 25;
/// A master's `create_doppelganger_on_death` that leaves ghosts.
pub const LEAVES_GHOSTS: u8 = 2;
/// The classes a building's watch leaves out (`FUN_1411cc980`) and the gates' class, whose annex
/// stays watched.
const UNWATCHED_CLASSES: [i16; 2] = [51, 54];
const GATE_CLASS: i16 = 39;
/// The master a building's watch leaves out (`FUN_1411cc980`).
const UNWATCHED_MASTER: i16 = 1808;
/// The originals whose ghost's leaving changes the local player's terrain (`FUN_141090ad0`).
const TERRAIN_ORIGINALS: [i16; 2] = [357, 1188];

/// What an entry of a watched list follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// A tile of the unified map (its index).
    Tile(usize),
    /// A building's own visibility mask (`+0x8f8`, the registry's), for one of type 50 and up
    /// with its footprint's UMV entry (`+0x904`).
    Mask(i32),
}

/// An entry of a player's watched list (32 bytes): the object, what it follows and the visible
/// and explored bits last seen there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watched {
    /// The object (`+0x18`, and its pointer `+0`).
    pub id: i32,
    /// What it follows (`+8`).
    pub cell: Cell,
    /// The bits last seen (`+0x10`).
    pub bits: u64,
}

/// A ghost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ghost {
    /// Its id (a reusable one, negative).
    pub id: i32,
    /// The object it shows while that one has not left (`+0x7c0`).
    pub original: Option<i32>,
    /// The original's id (`+0x7f8`, the save's `long_val`), −1 once it is gone.
    pub original_id: i32,
    /// The original's master (`+0x7d0`), while it is known.
    pub original_master: Option<i16>,
    /// The original's owner (`+0x7d8`, the save's `doppled_player`).
    pub original_owner: usize,
    /// Its place.
    pub position: [f32; 3],
    /// Its tile (`+0x7dc`, `+0x7e0`).
    pub tile: [i32; 2],
    /// The original's UMV entry (`+0x7f6`, the save's `Multi_Unified_Points`): the tiles of a
    /// building's footprint it follows besides its own; 0 for none.
    pub footprint: u8,
    /// The bits last seen (`+0x7e8`, not saved).
    pub bits: u64,
    /// The players who cannot see it (`+0x7f0`, the save's `CantSeeBits`).
    pub cant_see: u32,
    /// `+0x7f7`: its viewer saw its tile while the original's mask had not.
    pub seen: bool,
    /// Its state: 2, then 7 once destroyed, 8, and it leaves.
    pub state: u8,
}

impl Turns {
    /// The vision mask of player `p` (`player+0x2a8`, [`crate::vision`]).
    fn vision_mask(&self, p: usize) -> u64 {
        self.spatial.vision(p)[0]
    }

    /// The unified map's visible and explored bits at tile (`x`, `y`) (`map+0x5e30`, masked);
    /// 0 off the map or without the players' sight.
    fn tile_bits(&self, x: i32, y: i32) -> u64 {
        let Some(v) = self.spatial.sight.as_ref() else {
            return 0;
        };
        if x < 0 || y < 0 || v.width <= x || v.height <= y {
            return 0;
        }
        v.unified[(y * v.width + x) as usize] & MASK
    }

    /// The tile index of (`x`, `y`) in the unified map.
    fn tile_index(&self, x: i32, y: i32) -> Option<usize> {
        let v = self.spatial.sight.as_ref()?;
        (0 <= x && 0 <= y && x < v.width && y < v.height).then(|| (y * v.width + x) as usize)
    }

    /// What a cell shows now.
    fn cell_bits(&self, cell: Cell) -> Result<u64, Unsupported> {
        match cell {
            Cell::Tile(i) => Ok(self
                .spatial
                .sight
                .as_ref()
                .and_then(|v| v.unified.get(i))
                .map_or(0, |b| b & MASK)),
            Cell::Mask(id) => {
                let registry = self
                    .spatial
                    .sight
                    .as_ref()
                    .and_then(|v| v.registry.as_ref())
                    .ok_or(Unsupported(
                        "a watched building without the objects-in-sight registry",
                    ))?;
                Ok(registry.entries.get(&id).map_or(0, |e| e.mask) & MASK)
            }
        }
    }

    /// `FUN_141090430`: player `owner`'s watched list takes object `id` (an object of another
    /// owner, or one it holds already, is refused), at what `cell` shows now.
    pub(crate) fn watch(&mut self, owner: usize, id: i32, cell: Cell) -> Result<bool, Unsupported> {
        let holds = self.object(id).is_some_and(|(p, _)| p == owner);
        if !holds || self.players[owner].watched.iter().any(|w| w.id == id) {
            return Ok(false);
        }
        let bits = self.cell_bits(cell)?;
        self.players[owner].watched.push(Watched { id, cell, bits });
        Ok(true)
    }

    /// `FUN_141091310`: the watched list of player `owner` lets object `id` go (the last entry
    /// takes its place).
    pub(crate) fn unwatch(&mut self, owner: usize, id: i32) {
        let list = &mut self.players[owner].watched;
        if let Some(k) = list.iter().position(|w| w.id == id) {
            let _ = list.swap_remove(k);
        }
    }

    /// What a watched object of player `p` follows: a building of type 50 and up with its
    /// footprint's UMV entry its own mask, anything else its tile.
    fn cell_of(&self, p: usize, id: i32) -> Option<Cell> {
        let (_, u) = self.object(id)?;
        let kind = self
            .masters
            .get(p)
            .and_then(|m| m.get(u.life.master_id))
            .map_or(0, |m| m.kind);
        let umv = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.seer(id))
            .map_or(0, |s| s.umv);
        if 50 <= kind && umv != 0 {
            return Some(Cell::Mask(id));
        }
        let [x, y, _] = u.position;
        self.tile_index(x as i32, y as i32).map(Cell::Tile)
    }

    /// `FUN_1411cc980`: a building its owner watches: not of class 51 or 54, not desolid, not an
    /// annex (but a gate's), not master 1808.
    fn watches_building(&self, p: usize, id: i32) -> bool {
        let Some((_, u)) = self.object(id) else {
            return false;
        };
        let Some(m) = self.masters.get(p).and_then(|m| m.get(u.life.master_id)) else {
            return false;
        };
        !UNWATCHED_CLASSES.contains(&m.class)
            && !u.construction.is_some_and(|c| c.desolid)
            && (u.annex_of.is_none() || m.class == GATE_CLASS)
            && m.id != UNWATCHED_MASTER
    }

    /// A building whose construction started (`FUN_1411e5670`): its owner watches it
    /// ([`Turns::watches_building`]).
    pub(crate) fn watch_started(&mut self, p: usize, id: i32) -> Result<(), Unsupported> {
        if !self.watches_building(p, id) {
            return Ok(());
        }
        if let Some(cell) = self.cell_of(p, id) {
            let _ = self.watch(p, id, cell)?;
        }
        Ok(())
    }

    /// The watched lists the game's load builds, which no save holds: each player's objects in
    /// the load's order (its live list, then its sleeping list) join their owner's list, an
    /// object whose master leaves ghosts once it is dead (`FUN_14119df60`, at its tile) and a
    /// building its owner watches ([`Turns::watches_building`]; the load constructor
    /// `FUN_1411ac170`, at what it follows, an annex's link not known yet); then the rehook lets
    /// every annex but a gate's go (`TRIBE_Building_Object::rehook`). A building's mask is empty
    /// at a load.
    pub fn build_watched(&mut self) -> Result<(), Unsupported> {
        for p in 0..self.players.len() {
            self.players[p].watched.clear();
            let ids: Vec<i32> = self.players[p]
                .objects
                .iter()
                .chain(&self.players[p].sleeping)
                .map(|u| u.id)
                .collect();
            for id in ids {
                let Some((_, u)) = self.object(id) else {
                    continue;
                };
                let Some(m) = self.masters.get(p).and_then(|m| m.get(u.life.master_id)) else {
                    continue;
                };
                let (state, kind, ghosts, position) =
                    (u.life.state, m.kind, m.ghost_on_death, u.position);
                let (class, desolid, master) =
                    (m.class, u.construction.is_some_and(|c| c.desolid), m.id);
                if ghosts == LEAVES_GHOSTS
                    && 2 < state
                    && let Some(i) = self.tile_index(position[0] as i32, position[1] as i32)
                {
                    let _ = self.watch(p, id, Cell::Tile(i))?;
                }
                if kind == 80
                    && !UNWATCHED_CLASSES.contains(&class)
                    && !desolid
                    && master != UNWATCHED_MASTER
                    && let Some(cell) = self.cell_of(p, id)
                {
                    let _ = self.watch(p, id, cell)?;
                }
            }
            for list in [false, true] {
                let annexes: Vec<i32> = if list {
                    &self.players[p].sleeping
                } else {
                    &self.players[p].objects
                }
                .iter()
                .filter(|u| u.annex_of.is_some())
                .filter(|u| {
                    self.masters
                        .get(p)
                        .and_then(|m| m.get(u.life.master_id))
                        .is_some_and(|m| m.class != GATE_CLASS)
                })
                .map(|u| u.id)
                .collect();
                for id in annexes {
                    self.unwatch(p, id);
                }
            }
        }
        Ok(())
    }

    /// `FUN_14118e990`, at the death of object `id` of player `owner` whose master leaves ghosts
    /// (the static `set_state` 3, while the game state is not 2): every player who explored its
    /// tile but does not see it, Gaia aside, gets a ghost of it ([`Turns::make_ghost`]), in the
    /// players' order; the first bit past the players ends it. Then its owner watches its tile.
    pub(crate) fn ghosts_at_death(&mut self, owner: usize, id: i32) -> Result<(), Unsupported> {
        let Some((_, u)) = self.object(id) else {
            return Ok(());
        };
        if id < 0 {
            return Ok(());
        }
        let position = u.position;
        let (x, y) = (position[0] as i32, position[1] as i32);
        let bits = self.tile_bits(x, y);
        let (mut explored, mut visible) = (bits >> 32, bits & 0x1ff_ffff);
        let mut i = 0usize;
        while explored != 0 {
            if explored & 1 != 0 {
                if self.players.len() <= i {
                    break;
                }
                if visible & 1 == 0 && 0 < i {
                    self.make_ghost(i, position, id)?;
                }
            }
            i += 1;
            explored >>= 1;
            visible >>= 1;
        }
        if let Some(t) = self.tile_index(x, y) {
            let _ = self.watch(owner, id, Cell::Tile(t))?;
        }
        Ok(())
    }

    /// The viewer `viewer`'s ghost of object `original` at `at` (its DOPL master's slot 0xf8,
    /// `FUN_1410cfe80`): the pool of type 25 gives one back (`FUN_141546870`, its id kept), else a
    /// new one takes the world's next reusable id; it links its original (`FUN_14108f9b0`: its
    /// id, owner, master and a building's UMV entry; an original past state 6 is forgotten), joins
    /// its tile's list and its viewer's ghosts, and a twin of its viewer on the tile (a ghost of
    /// the same original in state 2) is destroyed (`FUN_1410928a0`; another viewer's twin and it
    /// note each other's player as one who cannot see it).
    pub(crate) fn make_ghost(
        &mut self,
        viewer: usize,
        at: [f32; 3],
        original: i32,
    ) -> Result<(), Unsupported> {
        if self.players.get(viewer).is_some_and(|p| p.computer) {
            return Err(Unsupported(
                "a computer player's ghost (its AI's notify 0x20d)",
            ));
        }
        let ghost_master = self.master_of(viewer, GHOST_MASTER)?;
        if !ghost_master.recyclable {
            return Err(Unsupported("a ghost master that is not recyclable"));
        }
        let (op, ou) = self
            .object(original)
            .ok_or(Unsupported("a ghost of an object the slice does not hold"))?;
        let (original_owner, original_state, original_master) =
            (op, ou.life.state, ou.life.master_id);
        let kind = self.master_of(op, original_master)?.kind;
        let umv = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.seer(original))
            .map_or(0, |s| s.umv);
        let id = if let Some(id) = self.recycling.ghosts.pop() {
            id
        } else {
            let id = self.recycling.next_id;
            self.recycling.next_id -= 1;
            id
        };
        let gone = 6 < original_state;
        let ghost = Ghost {
            id,
            original: (!gone).then_some(original),
            original_id: if gone { -1 } else { original },
            original_master: Some(original_master),
            original_owner,
            position: at,
            tile: [at[0] as i32, at[1] as i32],
            footprint: if 50 <= kind { umv } else { 0 },
            bits: 0,
            cant_see: 0,
            seen: false,
            state: 2,
        };
        self.spatial.insert_ghost(id, at);
        // the twin check over the tile's ghosts
        let mut cant_see = 0u32;
        for q in 0..self.players.len() {
            for k in 0..self.players[q].ghosts.len() {
                let g = self.players[q].ghosts[k];
                if g.tile != ghost.tile
                    || g.id == id
                    || g.original_id != ghost.original_id
                    || g.state != 2
                {
                    continue;
                }
                if q == viewer {
                    self.destroy_ghost(q, k);
                } else {
                    self.players[q].ghosts[k].cant_see |= 1 << viewer;
                    cant_see |= 1 << q;
                }
            }
        }
        self.players[viewer]
            .ghosts
            .push(Ghost { cant_see, ..ghost });
        Ok(())
    }

    /// The destroy (slot 0xd0, `FUN_14118f1e0`) of ghost `k` of player `p`: below state 7, its
    /// hit points go and it goes to state 7.
    fn destroy_ghost(&mut self, p: usize, k: usize) {
        let g = &mut self.players[p].ghosts[k];
        if g.state < 7 {
            g.state = 7;
        }
    }

    /// The ghosts of player `p` in their list's order (`FUN_141166550` on `+0x40`), each one's
    /// update (`FUN_141090000`): its static step (state 7 goes to 8, a ghost in state 8 leaves);
    /// one its viewer saw while the original's mask had not is destroyed; its original past state
    /// 6 is forgotten; then, its tile's bits (and its footprint's) changed, a ghost its viewer
    /// sees in state 2, unless one of the players sharing the viewer's sight does not, is
    /// destroyed (or marked seen, for a building's ghost whose original the viewer's mask misses
    /// and whose tile it does not see). A ghost that leaves (`FUN_141090ad0`) goes to the pool of
    /// type 25 and off its tile, the last ghost taking its place in the list. The viewer's
    /// `+0x2b0` table is not held (empty).
    pub(crate) fn update_ghosts(&mut self, p: usize) -> Result<(), Unsupported> {
        let mut k = 0;
        while k < self.players[p].ghosts.len() {
            let leaves = self.update_ghost(p, k)?;
            if leaves {
                let g = self.players[p].ghosts.swap_remove(k);
                if g.original_master
                    .is_some_and(|m| TERRAIN_ORIGINALS.contains(&m))
                {
                    return Err(Unsupported(
                        "a farm's ghost leaving (the local player's terrain, FUN_141092190)",
                    ));
                }
                self.spatial.remove_ghost(g.id, g.position);
                self.recycling.ghosts.push(g.id);
                continue;
            }
            k += 1;
        }
        Ok(())
    }

    fn update_ghost(&mut self, p: usize, k: usize) -> Result<bool, Unsupported> {
        let g = self.players[p].ghosts[k];
        let leaves = match g.state {
            7 => {
                self.players[p].ghosts[k].state = 8;
                false
            }
            8 => true,
            _ => false,
        };
        if g.seen && g.state < 3 {
            self.destroy_ghost(p, k);
            return Ok(leaves);
        }
        let alive = g
            .original
            .and_then(|o| self.object(o))
            .is_some_and(|(_, u)| u.life.state < 7);
        if !alive {
            let g = &mut self.players[p].ghosts[k];
            g.original = None;
            g.original_master = None;
            g.original_id = -1;
        }
        let g = self.players[p].ghosts[k];
        let [px, py, _] = g.position;
        let mut bits = self.tile_bits(g.tile[0], g.tile[1]);
        if g.footprint != 0 {
            let offsets = self
                .spatial
                .sight
                .as_ref()
                .and_then(|v| v.extents.get(usize::from(g.footprint) - 1))
                .map(|e| e.offsets.clone())
                .ok_or(Unsupported("a building's ghost without its UMV entry"))?;
            for [ox, oy] in offsets {
                bits |= self.tile_bits(px as i32 + ox, py as i32 + oy);
            }
        }
        if bits == g.bits {
            return Ok(leaves);
        }
        let mut unseen = false;
        let mut cant_see = g.cant_see;
        for i in 1..self.players.len() {
            let mask = self.vision_mask(i);
            if (mask >> g.original_owner) & 1 == 0 {
                if (mask >> p) & 1 != 0 && bits & mask == 0 {
                    unseen = true;
                }
            } else {
                cant_see |= 1 << p;
            }
        }
        let viewer = self.vision_mask(p);
        let mut seen = g.seen;
        let mut destroy = false;
        if bits & viewer != 0 && !unseen && g.state < 3 {
            if g.footprint != 0
                && let Some(o) = g.original
            {
                let original_mask = self.cell_bits(Cell::Mask(o))?;
                if original_mask & viewer == 0 && self.tile_bits(g.tile[0], g.tile[1]) & viewer == 0
                {
                    seen = true;
                }
            }
            destroy = !seen;
        }
        let g = &mut self.players[p].ghosts[k];
        g.seen = seen;
        g.cant_see = cant_see;
        g.bits = bits;
        if destroy {
            self.destroy_ghost(p, k);
        }
        Ok(leaves)
    }

    /// The watched list of player `p` after the player loop (`FUN_1410905e0`): an entry whose
    /// bits changed, of an object the world still holds, gives a ghost to every player, but its
    /// owner, who saw the cell and no longer does ([`Turns::make_ghost`], at the object's place),
    /// in the players' order, and takes the new bits; the last entry whose object is gone leaves
    /// the list (the last entry taking its place).
    pub(crate) fn watch_pass(&mut self, p: usize) -> Result<(), Unsupported> {
        let mut gone = None;
        for k in 0..self.players[p].watched.len() {
            let w = self.players[p].watched[k];
            let bits = self.cell_bits(w.cell)?;
            if bits == w.bits {
                continue;
            }
            match self.object(w.id) {
                None => gone = Some(w.id),
                Some((_, u)) => {
                    let at = u.position;
                    let mut lost = (((w.bits ^ bits) & w.bits) >> 1) & 0x7fff_ffff;
                    let mut i = 1usize;
                    while lost != 0 {
                        if lost & 1 != 0 && i != p {
                            self.make_ghost(i, at, w.id)?;
                        }
                        i += 1;
                        lost >>= 1;
                    }
                }
            }
            self.players[p].watched[k].bits = bits;
        }
        if let Some(id) = gone {
            self.unwatch(p, id);
        }
        Ok(())
    }
}
