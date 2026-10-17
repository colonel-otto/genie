# genie-save

[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-darkred?style=flat-square)](https://github.com/librematch/genie/blob/main/LICENSE-GPL-3.0)

The world save of Age of Empires II: Definitive Edition (save 66.x and later): the save-game file
header, then the world (`World::save` / `World::load_world`), the map and every player's objects
included. Three containers hold it: every chapter of a recorded game (genie-rec reads the
chapters), a saved game (`.aoe2spgame`, `.aoe2mpgame`), and the tick oracle's snapshots.

The layout (`src/generated/world.rs`) is generated from build 158041's load routines by the
workspace's `aoe2de-exe-re/layout` (`just layout`); do not edit it by hand. A parsed save writes
back byte for byte; genie-rec's corpus tests check both over every recording's chapters.

## License

[GPL-3.0](../../../LICENSE-GPL-3.0)
