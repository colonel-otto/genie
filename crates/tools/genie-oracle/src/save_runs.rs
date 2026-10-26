//! Runs from a tick oracle's save and their comparison with the game's: the players' objects,
//! sight and technologies, the unit groups, the hunted animals' origins.

use crate::masters::{MasterTable, TerrainTable};
use crate::{
    Decoder, Dumps, Error, ObjectRow, SavedStep, TickReport, frames, ghost_pool_from_dump, groups,
    inside_lists_from_dump, listed, mesh_from_dump, mesh_owners_from_dump, object_differences,
    recycle_pools_from_dump, registry_from_dump, run, slice, spatial, tile_differences,
    tile_lists_from_dump, visibility_differences, watch_at_load,
};
use genie_core::world::{Command, Player, Turns};
use genie_rec::de;
use genie_save::Header;

/// Every player's objects and sight after a run against the game's save `after`; with `tick`,
/// only the freezes of that update count, else all. A frozen object's differences are left out.
pub(super) fn compare_players(
    turns: &Turns,
    after: &SavedStep,
    tick: Option<u32>,
    report: &mut TickReport,
) -> Result<(), Error> {
    let seen = &after.seen;
    // the world's game state and the map's terrain go with Gaia's differences
    if turns.game_state != after.game.game_state {
        let d = format!(
            "game state slice {} game {}",
            turns.game_state, after.game.game_state
        );
        report.differences.push((0, d));
    }
    if let Some(t) = turns.spatial.terrain.as_ref() {
        for d in tile_differences(t, after.width, &after.terrain) {
            report.differences.push((0, d));
        }
    }
    for (p, player) in turns.players.iter().enumerate() {
        let id = p as u32;
        let now = |t: u32| tick.is_none_or(|n| n == t);
        if let Some((t, _, why)) = player
            .unsupported
            .clone()
            .filter(|u| player.stopped && now(u.0))
        {
            report.unsupported.push((id, None, t, why));
        }
        let frozen: Vec<i32> = player
            .objects
            .iter()
            .filter(|u| u.stopped)
            .map(|u| u.id)
            .chain(player.freezes.iter().map(|&(_, o, _)| o))
            .collect();
        for (t, o, why) in player.freezes.iter().filter(|f| now(f.0)) {
            report.unsupported.push((id, Some(*o), *t, why.clone()));
        }
        let rows: Vec<ObjectRow> = player
            .objects
            .iter()
            .filter(|u| listed(u))
            .map(ObjectRow::of_unit)
            .collect();
        let game = after.objects.get(p).ok_or(Error::NoSuchPlayer(id))?;
        if !player.stopped
            && let Some(r) = after.game.resources.get(p)
            && *r != player.resources
        {
            let d = format!("resources slice {:?} game {:?}", player.resources, r);
            report.differences.push((id, d));
        }
        // the other attributes, each one the slice holds that differs (reported apart)
        if !player.stopped
            && let Some(saved) = after.game.attributes.get(p)
        {
            report.attributes.extend(
                player
                    .attributes
                    .iter()
                    .zip(saved)
                    .enumerate()
                    .skip(4)
                    .filter(|(_, (a, b))| a.to_bits() != b.to_bits())
                    .map(|(i, (a, b))| (id, i, a.to_bits(), b.to_bits())),
            );
        }
        if let Some(&(status, resigned)) = after.game.statuses.get(p)
            && (status, resigned) != (player.game_status, player.resigned)
        {
            let d = format!(
                "game status slice {:?} game {:?} (status, resigned)",
                (player.game_status, player.resigned),
                (status, resigned)
            );
            report.differences.push((id, d));
        }
        if !player.stopped {
            for d in tech_differences(player, &after.game, p) {
                report.differences.push((id, d));
            }
        }
        if !player.stopped {
            for d in object_differences(&rows, game) {
                let object = d.split(':').next().and_then(|o| o.parse::<i32>().ok());
                if object.is_some_and(|o| frozen.contains(&o)) {
                    continue;
                }
                report.differences.push((id, d));
            }
        }
        if let Some(sight) = turns.spatial.sight.as_ref() {
            for d in visibility_differences(sight, seen, p) {
                report.differences.push((id, d));
            }
        }
    }
    Ok(())
}

/// A player's pending debit and technology states against the game's: per technology its state,
/// researches queued and done, work done and building.
pub(super) fn tech_differences(
    player: &Player,
    game: &genie_core::world::GameState,
    p: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(d) = game
        .pending_debit
        .get(p)
        .filter(|&&d| d != player.pending_debit)
    {
        out.push(format!(
            "pending debit slice {:?} game {:?}",
            player.pending_debit, d
        ));
    }
    let Some(theirs) = game.techs.get(p) else {
        return out;
    };
    let key = |t: &genie_core::technology::TechState| {
        (
            t.state,
            t.pending,
            t.times_researched,
            t.research_done,
            t.building_master,
        )
    };
    for (i, (a, b)) in player.techs.iter().zip(theirs).enumerate() {
        if key(a) != key(b) {
            out.push(format!(
                "tech {i}: slice {:?} game {:?} (state, queued, done, work, building)",
                key(a),
                key(b)
            ));
        }
    }
    out
}

/// The slice from a save: [`objects_from_save`]'s run, every player's state after it. Dev only.
///
/// # Errors
///
/// When a save or the recording does not parse.
///
/// [`objects_from_save`]: crate::save_objects::objects_from_save
pub fn run_from_save(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    ticks: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
) -> Result<Turns, Error> {
    from_save(save, recording, start_tick, ticks, masters, dumps, false)
}

/// The world's unit groups of a save, with the process globals dumped beside it when given.
///
/// # Errors
///
/// When the save does not parse or its groups do not import.
pub fn saved_groups(
    save: &[u8],
    globals: Option<&str>,
) -> Result<genie_core::groups::UnitGroups, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    let mut g = groups::unit_groups(&header.world.base_world).map_err(Error::Unmodelled)?;
    if let Some(text) = globals {
        groups::globals_from_dump(text, &mut g);
    }
    Ok(g)
}

/// The slice from a save after the command phase of the next update alone
/// ([`genie_core::world::Turns::command_phase`]), for a check against the game's save taken
/// there (the tick oracle's `snapshot_at` at `0x141552990`). Dev only.
///
/// # Errors
///
/// When a save or the recording does not parse.
pub fn command_phase_from_save(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
) -> Result<Turns, Error> {
    from_save(save, recording, start_tick, 1, masters, dumps, true)
}

/// [`run_from_save`]'s run; with `commands_only` the first update stops after its commands.
pub(super) fn from_save(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    ticks: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
    commands_only: bool,
) -> Result<Turns, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    let sections = header.players();
    let mut turns = slice(&header, masters);
    if let Some(text) = dumps.mesh {
        let m = mesh_from_dump(text).ok_or(Error::BadMeshDump)?;
        turns.pathing.set_mesh(m, mesh_owners_from_dump(text));
    }
    if let (Some(text), Some(sight)) = (dumps.registry, turns.spatial.sight.as_mut()) {
        sight.registry = Some(registry_from_dump(text)?);
    }
    if let Some(text) = dumps.tiles {
        let _ = turns.spatial.force_tile_lists(&tile_lists_from_dump(text)?);
    }
    if let Some(text) = dumps.pools {
        turns.recycling.missiles = recycle_pools_from_dump(text)?;
        turns.recycling.ghosts = ghost_pool_from_dump(text)?;
    }
    if let Some(text) = dumps.inside {
        let _ = turns.force_garrisons(&inside_lists_from_dump(text)?);
    }
    if let Some(text) = dumps.globals {
        groups::globals_from_dump(text, &mut turns.groups);
    }
    watch_at_load(&mut turns);
    let played = de::chapters(recording)?[0];
    hunted_origins(&mut turns, &played.header()?);
    let updates = start_tick + 1;
    turns.ticks = updates as u32;
    let all = frames(&played, updates + ticks)?;
    let next = all.get(updates..).unwrap_or_default();
    let mut decoder = Decoder::default();
    decoder.prime(&all[..updates.min(all.len())], &sections);
    if commands_only {
        if let Some(frame) = next.first() {
            let commands: Vec<Command> = frame
                .commands
                .iter()
                .filter_map(|p| decoder.command(p, &sections))
                .collect();
            turns.command_phase(frame.dt, &commands);
        }
        return Ok(turns);
    }
    run(&mut turns, &mut decoder, next, &sections);
    Ok(turns)
}

/// The hunted animals' unit AI fields the save does not hold (`TribeHuntedAnimalUnitAIModule`,
/// made with the unit: `FUN_140c11920`): the origin is the animal's place at the recording's
/// start (`start`, the recording's header save, whose load made the AI), the leash its master's
/// last task of action type 6's work range (the unit's `+0xb74`, `FUN_141206d50`).
pub(super) fn hunted_origins(turns: &mut Turns, start: &Header) {
    let places: std::collections::BTreeMap<i32, [f32; 3]> = start
        .players()
        .iter()
        .flat_map(|p| {
            p.live_objects()
                .chain(p.sleeping_objects())
                .collect::<Vec<_>>()
        })
        .filter_map(|o| o.static_part().map(|s| (s.id, [s.t_x, s.t_y, s.t_z])))
        .collect();
    for (p, player) in turns.players.iter_mut().enumerate() {
        let masters = turns.masters.get(p);
        for unit in player.objects.iter_mut().chain(player.sleeping.iter_mut()) {
            let Some(ai) = unit.ai.as_mut().filter(|ai| ai.ai_type == 2) else {
                continue;
            };
            let Some(&origin) = places.get(&unit.id) else {
                continue;
            };
            let leash = masters
                .and_then(|m| m.get(unit.life.master_id))
                .and_then(|m| m.tasks.iter().rev().find(|t| t.action_type == 6))
                .map_or(0.0, |t| t.work_range);
            ai.hunted = Some(genie_core::unit_ai::Hunted { origin, leash });
        }
    }
}

/// The world's objects as movement sees them, from a save ([`spatial`]), on a map without terrain.
pub fn spatial_from_save(save: &[u8]) -> Result<genie_core::map::spatial::Spatial, Error> {
    spatial_with_tiles(save, &TerrainTable::default())
}

/// The world's objects as movement sees them, from a save, with each tile's terrain class.
pub fn spatial_with_tiles(
    save: &[u8],
    terrain: &TerrainTable,
) -> Result<genie_core::map::spatial::Spatial, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    Ok(spatial(&header, terrain))
}
