use crate::event::{PlayEvent, Source};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct JellyfinPayload {
    #[serde(rename = "NotificationType")]
    pub notification_type: String,
    #[serde(rename = "Artist")]
    pub artist: Option<String>,
    #[serde(rename = "Album")]
    pub album: Option<String>,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "RunTimeTicks")]
    pub run_time_ticks: Option<u64>,
}

pub fn parse(payload: &JellyfinPayload) -> Result<PlayEvent> {
    if payload.notification_type != "PlaybackStopped" {
        return Err(anyhow!(
            "not a PlaybackStopped event: {}",
            payload.notification_type
        ));
    }

    Ok(PlayEvent {
        artist: payload.artist.clone().unwrap_or_default(),
        album: payload.album.clone(),
        track: payload.name.clone(),
        duration_secs: payload.run_time_ticks.map(|ticks| ticks / 10_000_000),
        played_at: Utc::now(),
        source: Source::Jellyfin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stopped_payload() -> JellyfinPayload {
        serde_json::from_str(r#"{
            "NotificationType": "PlaybackStopped",
            "Artist": "Portishead",
            "Album": "Dummy",
            "Name": "Sour Times",
            "RunTimeTicks": 2460000000
        }"#).unwrap()
    }

    #[test]
    fn parses_playback_stopped() {
        let event = parse(&stopped_payload()).unwrap();
        assert_eq!(event.artist, "Portishead");
        assert_eq!(event.album.as_deref(), Some("Dummy"));
        assert_eq!(event.track, "Sour Times");
        assert_eq!(event.duration_secs, Some(246));
        assert_eq!(event.source, Source::Jellyfin);
    }

    #[test]
    fn rejects_non_stopped_events() {
        let payload: JellyfinPayload = serde_json::from_str(r#"{
            "NotificationType": "PlaybackStart",
            "Name": "Track"
        }"#).unwrap();
        assert!(parse(&payload).is_err());
    }

    #[test]
    fn handles_missing_run_time_ticks() {
        let payload: JellyfinPayload = serde_json::from_str(r#"{
            "NotificationType": "PlaybackStopped",
            "Name": "Track"
        }"#).unwrap();
        let event = parse(&payload).unwrap();
        assert_eq!(event.duration_secs, None);
    }
}
