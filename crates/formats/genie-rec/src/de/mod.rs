//! Recorded games of Age of Empires II: Definitive Edition (save 66.x and later).
//!
//! The rest of this crate reads the older formats (AoK, AoC, HD). This module reads the current
//! one. Its layouts follow the workspace's reverse engineering of the 158041 engine and the
//! [aoe2rec](https://github.com/aoe2ct/aoe2rec) crate and ImHex patterns (MIT, see `NOTICE`).
