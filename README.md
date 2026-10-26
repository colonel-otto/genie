# genie

A deterministic reimplementation of the Age of Empires II: Definitive Edition engine as a Rust
library, plus readers and writers for the game's file formats.

Work in progress. Home: https://github.com/librematch/genie. See `NOTICE` for the origin and
licences of the crates.

## Why genie

Age of Empires II: Definitive Edition lives in a large ecosystem of tools around the game itself.
Modders change its units, technologies and civilizations with Advanced Genie Editor. CaptureAge
lets spectators and casters follow tournament matches with views the game does not offer.
Community sites read recorded games to build statistics, ratings and match analyses. All of these
tools need to understand the same rules and the same data as the game. Today each of them carries
its own partial copy of that knowledge, and often has to reverse-engineer the file formats again
before it can start on its actual purpose.

genie wants to be the one place where that knowledge lives: an open, deterministic implementation
of the game's engine that every tool can build on, and that the community maintains together. With
it, a tool can:

- **replay recorded games** and inspect the complete game state at any moment of a match, not only
  the players' commands that a recording stores;
- **analyse games** with exactly the rules the game uses, without running the game client;
- **train and evaluate AI agents**, with many headless simulations running in parallel on a server,
  or directly in the browser through WebAssembly;
- **share one source of truth** with modding tools such as Advanced Genie Editor and spectator
  tools such as CaptureAge, instead of every tool maintaining its own version of the rules.

genie uses a hexagonal (ports-and-adapters) architecture. At its centre is the simulation core: a
library that knows nothing about graphics, sound or input. Rendering, audio and user interfaces
plug into it through well-defined ports. A frontend can bring its own adapters, and genie can grow
its own over time, for example as `genie-ui` and `genie-audio` crates next to the core.

## Use cases

genie's first goal is to replay recorded games exactly. The use cases below build on that core and
are on the roadmap; none of them is available yet.

- **Optimising AI scripts with a coding agent.** genie runs the game's own `.per` AI scripts
  natively, for the latest game builds as well. A coding agent such as Codex or Claude Code edits a
  script, genie plays many headless AI-vs-AI matches with it, and returns the results, statistics,
  game-state data and recordings. The agent analyses them, improves the script and starts again.
  The scripts stay compatible with the original game: what genie runs, the game runs unchanged.
- **Learning agents.** Reinforcement-learning and neural-network agents, "AlphaStar for Age of
  Empires", play through genie's agent API and train against the game's scripts in thousands of
  parallel headless games. This is possible only in an engine like genie, not in the game itself.
- **Lookahead AIs.** genie copies a running game cheaply, so an AI can simulate several plans a few
  seconds ahead on other threads while the game goes on. For example, an AI switches every one to
  three seconds between three scripts (the HD AI, the DE AI and Immortal): it simulates each choice
  six seconds ahead with the opponents on Immortal, and keeps the one with the highest own score
  minus the opponents' average. The decision enters the game as ordinary commands, so the game
  stays deterministic and its recording replays without the lookahead.
- **Headless map generation.** From a random map script and a seed, genie generates the same map
  as the game, headless and in seconds: for minimaps of upcoming or past games and for analyses of
  how balanced the spawns are.

## Staying faithful to the game

The game's own engine is still being developed, and each release can change how the simulation
behaves. For now genie therefore follows the official engine release by release. It plays real
recorded games and compares its state with the checkpoints that the game wrote into each recording.
Every difference is a bug in genie, and fixing it brings genie back in line with the game. This is
what keeps the recorded games of every release playable.

This only holds for as long as the official engine receives updates. Once it no longer does, genie
does not need to follow it any more. The community can then continue to develop the engine on its
own, and fix its long-standing bugs instead of reproducing them.

## Documentation

- [`docs/ENGINE_SYSTEMS.md`](docs/ENGINE_SYSTEMS.md) explains how the simulation core is
  organised: one tick, one object's update, the engine's systems, and how to find a behaviour.
  Each folder of the core's source has a `README.md` for its system.
- [`docs/DEBUGGING.md`](docs/DEBUGGING.md) shows how to play a recording, find where it goes
  wrong, and report a bug.

## To the developers of the game

We build genie for the community first, but we also build it with you in mind: Forgotten Empires,
the CaptureAge team, and every other studio that works on the game.

What we hope to offer you is an engine that you do not have to carry alone. While your in-house
engine is still being developed, genie follows it and stays compatible with each of your releases.
If you ever stop developing it, genie can carry it on: an engine that stays close to the community,
that the community maintains, and that is checked against real recorded games. In the long run,
that means more development capacity for the game, without new long-term costs for you.

The engine crates are licensed under AGPL-3.0. If that licence does not fit the game or CaptureAge,
ask us. We can license the engine to you separately for that purpose. You can then ship genie as a
library in your installer, for example as a DLL that the game links to. CaptureAge has a second way
that needs no separate licence: it can talk to genie as a separate program over gRPC on localhost,
as it talks to the game today.

We want to work with you, not around you. If you have questions, wishes or concerns, please start a discussion at https://github.com/librematch/genie/discussions.
