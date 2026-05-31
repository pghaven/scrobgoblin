use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct PlayEvent {
    pub artist: String,
    pub album: Option<String>,
    pub track: String,
    pub duration_secs: Option<u64>,
    pub played_at: DateTime<Utc>,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    Navidrome,
    Plex,
    Jellyfin,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Navidrome => write!(f, "Navidrome"),
            Source::Plex => write!(f, "Plex"),
            Source::Jellyfin => write!(f, "Jellyfin"),
        }
    }
}
