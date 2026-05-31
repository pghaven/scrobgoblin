use crate::event::{PlayEvent, Source};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct PlexPayload {
    pub event: String,
    #[serde(rename = "Metadata")]
    pub metadata: Option<PlexMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct PlexMetadata {
    #[serde(rename = "grandparentTitle")]
    pub grandparent_title: Option<String>,
    #[serde(rename = "parentTitle")]
    pub parent_title: Option<String>,
    pub title: String,
    pub duration: Option<u64>,
}

pub fn parse(payload: &PlexPayload) -> Result<PlayEvent> {
    if payload.event != "media.scrobble" {
        return Err(anyhow!("not a scrobble event: {}", payload.event));
    }
    let meta = payload
        .metadata
        .as_ref()
        .ok_or_else(|| anyhow!("missing Metadata"))?;

    Ok(PlayEvent {
        artist: meta.grandparent_title.clone().unwrap_or_default(),
        album: meta.parent_title.clone(),
        track: meta.title.clone(),
        duration_secs: meta.duration.map(|ms| ms / 1000),
        played_at: Utc::now(),
        source: Source::Plex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scrobble_payload() -> PlexPayload {
        serde_json::from_str(r#"{
            "event": "media.scrobble",
            "Metadata": {
                "grandparentTitle": "Radiohead",
                "parentTitle": "OK Computer",
                "title": "Karma Police",
                "duration": 264000
            }
        }"#).unwrap()
    }

    #[test]
    fn parses_scrobble_event() {
        let event = parse(&scrobble_payload()).unwrap();
        assert_eq!(event.artist, "Radiohead");
        assert_eq!(event.album.as_deref(), Some("OK Computer"));
        assert_eq!(event.track, "Karma Police");
        assert_eq!(event.duration_secs, Some(264));
        assert_eq!(event.source, Source::Plex);
    }

    #[test]
    fn rejects_non_scrobble_events() {
        let payload: PlexPayload = serde_json::from_str(r#"{
            "event": "media.play",
            "Metadata": { "title": "Track", "grandparentTitle": "Artist" }
        }"#).unwrap();
        assert!(parse(&payload).is_err());
    }
}
