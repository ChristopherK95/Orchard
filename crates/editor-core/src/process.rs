//! Spawning helpers shared by everything the core runs (the ACP adapter, git, version checks).

use std::ffi::OsStr;

use tokio::process::Command;

/// A command that won't flash a console window up from the GUI app on Windows.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}
