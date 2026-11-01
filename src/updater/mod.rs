//! FFmpeg build discovery (BtbN GitHub releases or gyan.dev) and update logic.
//!
//! * `sources` – pure parsing of the GitHub API JSON / gyan version files
//! * `install` – download, SHA-256 verify, extract, swap-in with backup, roll back

pub mod install;
pub mod sources;

use serde::{Deserialize, Serialize};

pub use sources::{AvailableBuild, ArchiveKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UpdateSource {
    /// github.com/BtbN/FFmpeg-Builds (win64 GPL static zip, SHA-256 from the GitHub API)
    #[default]
    BtbN,
    /// gyan.dev essentials build (zip for release, 7z for git; .sha256 files)
    Gyan,
}

impl UpdateSource {
    pub const ALL: [UpdateSource; 2] = [UpdateSource::BtbN, UpdateSource::Gyan];
    pub fn label(self) -> &'static str {
        match self {
            UpdateSource::BtbN => "BtbN (GitHub) – win64 GPL",
            UpdateSource::Gyan => "gyan.dev – essentials",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UpdateChannel {
    /// Newest release branch (e.g. n9.0 / 9.0.2)
    #[default]
    Stable,
    /// Git master nightly
    Latest,
}

impl UpdateChannel {
    pub const ALL: [UpdateChannel; 2] = [UpdateChannel::Stable, UpdateChannel::Latest];
    pub fn label(self) -> &'static str {
        match self {
            UpdateChannel::Stable => "Stable release",
            UpdateChannel::Latest => "Latest (git master)",
        }
    }
}

/// Query the network for the newest build of the chosen source/channel.
pub fn check(source: UpdateSource, channel: UpdateChannel) -> Result<AvailableBuild, String> {
    match source {
        UpdateSource::BtbN => {
            let json = install::fetch_text(sources::BTBN_API)?;
            sources::parse_btbn_release(&json, channel)
        }
        UpdateSource::Gyan => {
            let (ver_url, sha_url) = match channel {
                UpdateChannel::Stable => (sources::GYAN_RELEASE_VERSION, sources::GYAN_RELEASE_SHA),
                UpdateChannel::Latest => (sources::GYAN_GIT_VERSION, sources::GYAN_GIT_SHA),
            };
            let ver = install::fetch_text(ver_url)?;
            let sha = install::fetch_text(sha_url).ok();
            sources::gyan_build(channel, &ver, sha.as_deref())
        }
    }
}
