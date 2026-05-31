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

#[derive(Debug, Clone)]
pub struct NowPlayingEvent {
    pub artist: String,
    pub album: Option<String>,
    pub track: String,
    pub duration_secs: Option<u64>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_playing_event_source_display() {
        let e = NowPlayingEvent {
            artist: "Radiohead".to_string(),
            album: Some("OK Computer".to_string()),
            track: "Karma Police".to_string(),
            duration_secs: Some(264),
            source: Source::Navidrome,
        };
        assert_eq!(format!("{}", e.source), "Navidrome");
    }
}
