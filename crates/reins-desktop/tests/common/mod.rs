//! The `reins` binary these tests launch. Nextest sets `NEXTEST_BIN_EXE_reins` to the relocated
//! path when a run is restored from an archive; `cargo test` sets `CARGO_BIN_EXE_reins`.

use std::path::PathBuf;

pub fn reins_exe() -> PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_reins")
        .or_else(|| std::env::var_os("CARGO_BIN_EXE_reins"))
        .map(PathBuf::from)
        .expect("reins binary")
}
