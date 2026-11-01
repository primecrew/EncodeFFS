//! Pure command builders. Nothing in here touches the file system or spawns
//! processes: each builder takes a parameter struct and returns a `BuiltCommand`
//! (ffmpeg argv + any side files that must be written before running).
//!
//! One module per mode so new modes can be added without touching the others.

pub mod common;
pub mod chapters;
pub mod reencode;   // mode 1 + mode 7 (batch uses the same builder per file)
pub mod still;      // mode 2
pub mod commentary; // mode 3
pub mod extract;    // mode 4
pub mod replace_audio; // mode 5
pub mod mix;        // mode 6
pub mod batch;      // mode 7 planning
pub mod subs;       // mode 8
pub mod remux;      // mode 9

use std::path::PathBuf;

/// The result of a command builder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuiltCommand {
    /// Arguments for ffmpeg (program name not included).
    pub args: Vec<String>,
    /// Files that must be written before ffmpeg runs (path, contents), e.g. FFMETADATA chapters.
    pub side_files: Vec<(PathBuf, String)>,
    /// Non-fatal warnings to show the user in the preview.
    pub warnings: Vec<String>,
}

/// Small helper so builders can write `a.push_all(["-c:v", "copy"])`.
pub(crate) trait ArgVec {
    fn push_all<I, S>(&mut self, it: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>;
}
impl ArgVec for Vec<String> {
    fn push_all<I, S>(&mut self, it: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for s in it {
            self.push(s.into());
        }
    }
}

/// Convert a path to an ffmpeg argument. Paths are passed as separate argv
/// entries so spaces/unicode need no quoting.
pub(crate) fn p(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}
