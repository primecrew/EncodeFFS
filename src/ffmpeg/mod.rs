//! Everything that talks to ffmpeg/ffprobe processes.

pub mod locate;
pub mod managed;
pub mod probe;
pub mod progress;
pub mod runner;
pub mod version;
pub mod avisynth;

use std::path::Path;
use std::process::Command;

/// Windows: don't flash a console window for child processes.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A `Command` that never pops up a console window on Windows.
pub fn hidden_command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}
