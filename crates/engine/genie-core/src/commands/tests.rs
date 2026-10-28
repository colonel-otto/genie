//! The command enum's issuing player and the lane a failed run stops.

use crate::GenieError;
use crate::commands::{
    AllBackToWorkCommand, AttackMoveCommand, AutoScoutCommand, BuildCommand, BuildWallCommand,
    BuildingQueueCommand, Command, DeleteCommand, FlareCommand, GameCommand, GateCommand,
    GatherPointCommand, GoBackToWorkCommand, OrderCommand, OrderUnits, PatrolCommand, QueueCommand,
    ResearchCommand, ResignCommand, SeekShelterCommand, StopCommand, TownBellCommand, TradeCommand,
    TransformObjectCommand, TributeCommand, UnitAiStateCommand, UnitOrderAction, UnitOrderCommand,
    UnloadCommand,
};

/// One command of each kind, issued by player `p`.
fn one_of_each(p: usize) -> Vec<Command> {
    let ids = vec![7];
    let gift = crate::players::tribute::Gift {
        attribute: 0,
        amount: 0.0,
        fee: 0.0,
    };
    vec![
        Command::Build(BuildCommand {
            player: p,
            master: 70,
            position: [0.0; 3],
            builders: ids.clone(),
            queued: false,
            commits: true,
            frame: 0,
        }),
        Command::BuildWall(BuildWallCommand {
            player: p,
            master: 72,
            from: [0, 0],
            to: [1, 1],
            target: -1,
            builders: ids.clone(),
            queued: false,
            commits: true,
        }),
        Command::GatherPoint(GatherPointCommand {
            player: p,
            ids: ids.clone(),
            point: [0.0; 2],
            target: -1,
            target_master: -1,
            own: false,
        }),
        Command::Queue(QueueCommand {
            player: p,
            buildings: ids.clone(),
            unit: 83,
            amount: 1,
            button: -1,
        }),
        Command::Research(ResearchCommand {
            player: p,
            building: 7,
            count: 1,
            buildings: vec![],
            tech: 22,
            target: -1,
            shift: false,
        }),
        Command::Unmodelled {
            player: p,
            why: GenieError::unsupported("a test's"),
        },
        // a sub-command for no player of the world
        Command::Game(GameCommand {
            issuer: p,
            player: -1,
            ..GameCommand::default()
        }),
        Command::CommandQueue(BuildingQueueCommand {
            player: p,
            ids: ids.clone(),
            target: -1,
            point: [0.0; 2],
        }),
        Command::GoBackToWork(GoBackToWorkCommand {
            player: p,
            ids: ids.clone(),
        }),
        Command::AllBackToWork(AllBackToWorkCommand { player: p, id: 7 }),
        Command::TownBell(TownBellCommand {
            player: p,
            id: 7,
            activate: true,
        }),
        Command::Delete(DeleteCommand { player: p, id: 7 }),
        Command::Flare(FlareCommand {
            player: p,
            target: -1,
            point: [0.0; 2],
            players: 2,
        }),
        Command::Tribute(TributeCommand {
            player: p,
            to: 1,
            gifts: [gift; 4],
        }),
        Command::Trade(TradeCommand {
            player: p,
            attribute: 0,
            times: 1,
            buy: true,
        }),
        Command::Gate(GateCommand { player: p, id: 7 }),
        Command::Unload(UnloadCommand {
            player: p,
            ids: ids.clone(),
            point: [-1.0; 2],
            flag: 0,
            unit_type: -1,
            sound: false,
        }),
        Command::TransformObject(TransformObjectCommand {
            player: p,
            id: 7,
            master: 83,
        }),
        Command::Resign(ResignCommand {
            player: p,
            dropped: false,
        }),
        Command::AutoScout(AutoScoutCommand {
            player: p,
            ids: ids.clone(),
        }),
        Command::Stop(StopCommand {
            player: p,
            ids: ids.clone(),
        }),
        Command::Patrol(PatrolCommand {
            player: p,
            ids: ids.clone(),
            waypoints: vec![[1.0, 1.0]],
            reset: false,
        }),
        Command::AttackMove(AttackMoveCommand {
            player: p,
            ids: ids.clone(),
            waypoints: vec![[1.0, 1.0]],
            reset: false,
            queued: false,
            ignore_pending: false,
        }),
        Command::SeekShelter(SeekShelterCommand {
            player: p,
            ids: ids.clone(),
        }),
        Command::UnitAiState(UnitAiStateCommand {
            player: p,
            ids: ids.clone(),
            state: 1,
        }),
        Command::UnitOrder(UnitOrderCommand {
            player: p,
            ids: ids.clone(),
            target: -1,
            through_ai: false,
            nearby: false,
            action: UnitOrderAction::DropOff,
            param: 0,
            human: false,
        }),
        Command::Order(OrderCommand {
            player: p,
            units: OrderUnits::Listed(ids),
            order: None,
        }),
    ]
}

/// Every command names its issuing player (GAME its issuer), and a failed run stops that
/// player's lane, but a GAME sub-command's, which stops its target's (or its issuer's for a
/// target outside the world).
#[test]
fn a_command_names_the_player_whose_lane_it_stops() {
    let commands = one_of_each(3);
    assert_eq!(commands.len(), 27);
    for c in &commands {
        assert_eq!((c.player(), c.stopped_player(8)), (3, 3), "{c:?}");
    }
    let game = Command::Game(GameCommand {
        issuer: 3,
        player: 5,
        ..GameCommand::default()
    });
    assert_eq!((game.player(), game.stopped_player(8)), (3, 5));
    assert_eq!(game.stopped_player(4), 3);
}
