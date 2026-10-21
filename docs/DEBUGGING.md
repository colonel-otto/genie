# Debugging and reporting bugs

genie plays recorded games and compares its state with the checkpoints that the game wrote into
each recording. Every difference is a bug in genie. This page shows how to find where a recording
goes wrong, and what a bug report needs. Open bug reports at
https://github.com/librematch/genie/issues.

## What genie can play today

genie plays the recordings of one engine build: build 158041, game version `101.103.26969.0`. The
game puts its version into the name of each recording, for example
`SP Replay v101.103.26969.0 @….aoe2record`. genie cannot read the recordings of other builds yet.
That read error is not a bug.

genie also needs the game's data file of the same build: `resources/_common/dat/empires2_x2_p1.dat`
from the game files of Steam build 20295388. Leave `dropsites.json` and `linkedTechs.json` beside
it, where the game installs them. genie reads both from the directory of the data file.

## 1. Play the recording

Run the census example from the root of the repository:

```sh
cargo run --release -p genie-oracle --example census -- REC DAT [UPDATES]
```

`REC` is the recording and `DAT` is the data file. `UPDATES` limits the run to that many updates.

The census plays the recording from its start, with nothing forced. It compares each checkpoint
with genie's state. If every player stops or differs, the census ends early. Then it prints
one line per player, Gaia first:

```text
1759 updates (AllDone), 3 checkpoints
player 0: stops at update 1254: an owner search near a stopped unit that counts; 0 checkpoints hold; no checkpoint differs
player 1: stops at update 1759: an object of a stopped player may be near; 3 checkpoints hold; no checkpoint differs
player 2: stops at update 1154: a command the slice does not model: TribeGiveAttributes; 2 checkpoints hold; no checkpoint differs
```

A player's line can report two findings:

- `stops at update N: REASON`: at update N, the player met a case that genie does not model yet.
  From then on, genie no longer runs the player's objects. `REASON` names the case.
- `the checkpoint after N syncs differs in words [...]`: the player still ran, but genie's state
  differs from the game's. Each entry is `(word, genie's value, the game's value)` of the 11-word
  checkpoint block.

The checkpoints example prints every checkpoint up to `UPDATES`, for every player:

```sh
cargo run --release -p genie-oracle --example checkpoints -- REC DAT UPDATES
```

```text
checkpoint at 499: p1 ok p2 ok
checkpoint at 999: p1 ok p2 ok
checkpoint at 1499: p1 ok p2 [(7, 171105, 171448), (9, 16205, 16368)]
```

This example does not know where a player stops. In the run above, player 2 stopped at update
1154. Its difference at 1499 is a result of that stop, not a second bug.

## 2. Decide what you found

| What you see                                                 | What it means                                    | Report it                                   |
| ------------------------------------------------------------ | ------------------------------------------------ | ------------------------------------------- |
| A checkpoint differs, and the player did not stop before it  | genie computes a different state than the game   | Yes. These reports are the most important.  |
| A player stops                                               | genie does not model that case yet               | Yes, if no open issue names the same reason |
| genie panics                                                 | a bug in genie                                   | Yes, with the backtrace                     |
| A recording of build 158041 does not read                    | a bug in the format crates (genie-rec, genie-save) | Yes                                         |
| A checkpoint differs after the player stopped                | a result of the stop                             | No. Report the stop.                        |
| A recording of a different build does not read               | genie does not support that build yet            | No                                          |

The reports of stops tell us which cases block the most recordings, so that we can model those
first.

Before you open an issue, search the issues for the reason text. Each reason is a string in
genie-core's source, so `grep -rn "REASON" crates/engine` finds the code that stops.

## 3. Record the engine's log

The census tells where a run goes wrong. The engine's log tells what led up to it. A log is most
useful for a checkpoint that differs.

genie-core writes its log through [`tracing`](https://docs.rs/tracing). If `RUST_LOG` holds a
filter, the census and checkpoints examples print it to stderr as text lines. Without `RUST_LOG`,
they print only their results.

1. Build the example first, so that cargo's own messages stay out of the log:

   ```sh
   cargo build --release -p genie-oracle --example census
   ```

2. Set `RUST_LOG`, and write stderr to a file:

   ```sh
   RUST_LOG=genie_core::unsupported=warn,genie_core::state=debug cargo run --release -q -p genie-oracle --example census -- REC DAT 2> genie.log
   ```

Each line of the log names its target and its fields:

```text
 WARN genie_core::unsupported: a player stops tick=1154 player=2 why="a command the slice does not model: TribeGiveAttributes"
```

To follow one object, search the log for `object=ID`. The lines carry no time, so two runs of one
recording give the same log. You can compare two logs line by line, for example before and after a
change to genie. If `RUST_LOG` does not parse, the example stops with an error that names the value.

genie-core itself installs no subscriber. In a program of your own, `RUST_LOG` does nothing until
the program installs one. `crates/tools/genie-oracle/examples/support/mod.rs` shows how.

### What the log contains

genie-core logs under three targets. `crates/engine/genie-core/src/trace.rs` defines them.

| Target                    | Level        | Content                                                                                                                                                                                                                                                                    |
| ------------------------- | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `genie_core::unsupported` | WARN         | Where a case that genie does not model lands: `an object freezes` (with `object`), `a player stops`, or `a case not modelled` (a player's first such case, without a freeze). Each event carries `tick`, `player` and `why`, the reason text.                               |
| `genie_core::state`       | DEBUG        | An object's update changed its state (`object_state`: `from`, `to`), its top action (`action`: `from`, `to`, `from_state`, `to_state`) or its unit AI (`unit_ai`: `from_order`, `to_order`, `from_ai_action`, `to_ai_action`, `from_target`, `to_target`). Each event carries `tick`, `player` and `object`. `-1` stands for none. |
| `genie_core::update`      | DEBUG, TRACE | The span `update` (`tick`, `world_time`) of each world update, the event `command` (`player`, `command`) of each recorded command, and the span `player` (`player`) of each player's update, all at DEBUG. The span `object` (`id`, `master`) of each object's update, at TRACE. |

Useful values of `RUST_LOG`:

| `RUST_LOG`                                                | What you get                                                   |
| --------------------------------------------------------- | -------------------------------------------------------------- |
| `genie_core::unsupported=warn`                            | Only the cases that genie does not model. The log stays small. |
| `genie_core::state=debug`                                 | Every state change of every object                             |
| `genie_core::unsupported=warn,genie_core::state=debug`    | Both. Separate several targets with commas.                    |
| `genie_core=debug`                                        | Everything except the object spans                             |
| `genie_core=trace`                                        | Everything, with one span per object and update                |

## 4. Write the report

Open the [bug report form](https://github.com/librematch/genie/issues/new?template=bug_report.yml).

Use the reason, the difference or the panic message as the title, word for word. For example:
`player 2 stops at update 1154: a command the slice does not model: TribeGiveAttributes`. The next
person with the same problem then finds your issue. Open one issue per reason or difference.

The form asks for everything that we need to play the recording as you did. Collect it first:

- The recording. Pack it into a `.zip`, because GitHub does not accept `.aoe2record` files as
  attachments. If you cannot share it, say so.
- If you know it: the game version of the recording.
- genie's commit (`git rev-parse --short HEAD`). If you changed genie, say what you changed
  (`git status`, `git diff`).
- The command that you ran, with all its arguments.
- The full output of the census or checkpoints example.
- If you recorded a log: the log as a file, with the `RUST_LOG` value that you used.
- For a panic: the backtrace. Run the command again with `RUST_BACKTRACE=1` to get it.
- If you watched the recording in the game: what happened there at that point.
