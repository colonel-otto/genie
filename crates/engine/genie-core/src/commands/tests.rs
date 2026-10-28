//! The command enum's issuing player and the lane a failed run stops.

use crate::GenieError;
use crate::commands::{
    AllBackToWorkCommand, AttackMoveCommand, AutoScoutCommand, BuildCommand, BuildWallCommand,
    BuildingQueueCommand, Command, DeleteCommand, FlareCommand, GameCommand, GateCommand,
    GatherPointCommand, GoBackToWork2Command, GoBackToWorkCommand, Order, OrderCommand,
    PatrolCommand, QueueCommand, ResearchCommand, ResignCommand, SeekShelterCommand, StopCommand,
    TownBellCommand, TradeCommand, TransformObjectCommand, TributeCommand, UnitAiStateCommand,
    UnitList, UnitOrderAction, UnitOrderCommand, UnloadCommand, UnmodelledCommand, Waypoints,
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
            position: [0.0; 2],
            target: -1,
            sprite_override: -1,
            builders: UnitList::of(ids.clone()),
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
            builders: UnitList::of(ids.clone()),
            queued: false,
            commits: true,
        }),
        Command::GatherPoint(GatherPointCommand {
            player: p,
            units: UnitList::of(ids.clone()),
            point: [0.0; 2],
            target: -1,
            target_master: -1,
            own: false,
        }),
        Command::Queue(QueueCommand {
            player: p,
            buildings: UnitList::of(ids.clone()),
            building_master: -1,
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
        Command::Unmodelled(UnmodelledCommand {
            player: p,
            packet: Vec::new(),
            why: GenieError::unsupported("a test's"),
        }),
        // a sub-command for no player of the world
        Command::Game(GameCommand {
            issuer: p,
            player: -1,
            ..GameCommand::default()
        }),
        Command::CommandQueue(BuildingQueueCommand {
            player: p,
            units: UnitList::of(ids.clone()),
            target: -1,
            point: [0.0; 2],
        }),
        Command::GoBackToWork(GoBackToWorkCommand { player: p, id: 7 }),
        Command::GoBackToWork2(GoBackToWork2Command {
            player: p,
            units: UnitList::of(ids.clone()),
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
            shown: Vec::new(),
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
            market: -1,
            buy: true,
        }),
        Command::Gate(GateCommand { player: p, id: 7 }),
        Command::Unload(UnloadCommand {
            player: p,
            units: UnitList::of(ids.clone()),
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
            units: UnitList::of(ids.clone()),
        }),
        Command::Stop(StopCommand {
            player: p,
            units: UnitList::of(ids.clone()),
        }),
        Command::Patrol(PatrolCommand {
            player: p,
            units: UnitList::of(ids.clone()),
            waypoints: Waypoints::of(&[[1.0, 1.0]]),
            reset: false,
        }),
        Command::AttackMove(AttackMoveCommand {
            player: p,
            units: UnitList::of(ids.clone()),
            waypoints: Waypoints::of(&[[1.0, 1.0]]),
            reset: false,
            queued: false,
            ignore_pending: false,
        }),
        Command::SeekShelter(SeekShelterCommand {
            player: p,
            units: UnitList::of(ids.clone()),
        }),
        Command::UnitAiState(UnitAiStateCommand {
            player: p,
            units: UnitList::of(ids.clone()),
            state: 1,
        }),
        Command::UnitOrder(UnitOrderCommand {
            player: p,
            units: UnitList::of(ids.clone()),
            target: -1,
            point: [-1.0, -1.0],
            q_id: -1,
            through_ai: false,
            nearby: false,
            action: UnitOrderAction::DropOff,
            param: 0,
            human: false,
        }),
        Command::Order(OrderCommand {
            player: p,
            units: UnitList::of(ids),
            order: Order::Retreat(crate::orders::Interact::default()),
        }),
    ]
}

/// Every command names its issuing player (GAME its issuer), and a failed run stops that
/// player's lane, but a GAME sub-command's, which stops its target's (or its issuer's for a
/// target outside the world).
#[test]
fn a_command_names_the_player_whose_lane_it_stops() {
    let commands = one_of_each(3);
    assert_eq!(commands.len(), 28);
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

/// The waypoints the handlers read: the first of their count, none without 1 to 10.
#[test]
fn a_patrol_reads_one_to_ten_waypoints() {
    let two = Waypoints::of(&[[1.0, 2.0], [3.0, 4.0]]);
    assert_eq!(two.used(), Some(&[[1.0, 2.0], [3.0, 4.0]][..]));
    assert_eq!(Waypoints::default().used(), None);
    let eleven = Waypoints { count: 11, ..two };
    assert_eq!(eleven.used(), None);
}

/// The market trades the attribute's low byte, food, wood and stone only.
#[test]
fn a_trade_of_another_commodity_does_nothing() {
    let player = crate::world::Player::default();
    let mut w = crate::world::World::new(0, 1, 1, vec![0; 2], vec![player; 2]);
    let before = w.clone();
    for attribute in [3, 0x0103] {
        TradeCommand {
            player: 1,
            attribute,
            times: 1,
            market: -1,
            buy: true,
        }
        .run(&mut w);
    }
    assert_eq!(w, before);
}
