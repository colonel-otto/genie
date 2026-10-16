# genie-swiss-knife

`gsk`, the genie swiss knife: one command-line tool for the files of the Genie engine, the engine of
Age of Empires II: Definitive Edition and its predecessors.

**Status: placeholder.** This release reserves the crate name. The binary `gsk` does nothing yet and
exits with an error.

## Plan

gsk will sit on top of the format crates of [genie](https://github.com/librematch/genie). These
crates read and write the game's files, and gsk makes them usable without writing Rust:

- the verb comes first, the format comes from the file: `gsk info <file>`, `gsk extract <file>`,
  `gsk convert <file> <out>`, `gsk roundtrip <file>`
- `--format` names the format where the file does not tell it
- text output for people, `--json` for scripts

gsk is built on the [Abscissa](https://github.com/iqlusioninc/abscissa) application framework.

```sh
cargo install genie-swiss-knife   # installs the command `gsk`
```

## Licence

[AGPL-3.0-only](https://github.com/librematch/genie/blob/main/LICENSE-AGPL-3.0)

Age of Empires II © Microsoft Corporation. This project is not endorsed by or affiliated with
Microsoft.
