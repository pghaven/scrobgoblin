use crate::event::{NowPlayingEvent, PlayEvent, Source};
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
    #[serde(rename = "PlaybackPositionTicks")]
    pub played_position_ticks: Option<u64>,
}

pub fn parse(payload: &JellyfinPayload) -> Result<PlayEvent> {
    if payload.notification_type != "PlaybackStop" {
        return Err(anyhow!(
            "not a PlaybackStop event: {}",
            payload.notification_type
        ));
    }
    if payload.played_position_ticks == Some(0) {
        return Err(anyhow!(
            "not a PlaybackStop event: position 0 (session cleanup)"
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

pub fn parse_now_playing(payload: &JellyfinPayload) -> Result<NowPlayingEvent> {
    if payload.notification_type != "PlaybackStart" {
        return Err(anyhow!(
            "not a PlaybackStart event: {}",
            payload.notification_type
        ));
    }

    Ok(NowPlayingEvent {
        artist: payload.artist.clone().unwrap_or_default(),
        album: payload.album.clone(),
        track: payload.name.clone(),
        duration_secs: payload.run_time_ticks.map(|ticks| ticks / 10_000_000),
        source: Source::Jellyfin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stopped_payload() -> JellyfinPayload {
        serde_json::from_str(
            r#"{
            "NotificationType": "PlaybackStop",
            "Artist": "Portishead",
            "Album": "Dummy",
            "Name": "Sour Times",
            "RunTimeTicks": 2460000000,
            "PlaybackPositionTicks": 2460000000
        }"#,
        )
        .unwrap()
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
    fn rejects_position_zero_stop() {
        let payload: JellyfinPayload = serde_json::from_str(
            r#"{
            "NotificationType": "PlaybackStop",
            "Artist": "Portishead",
            "Name": "Sour Times",
            "RunTimeTicks": 2460000000,
            "PlaybackPositionTicks": 0
        }"#,
        )
        .unwrap();
        assert!(parse(&payload).is_err());
    }

    #[test]
    fn rejects_non_stopped_events() {
        let payload: JellyfinPayload = serde_json::from_str(
            r#"{
            "NotificationType": "PlaybackStart",
            "Name": "Track"
        }"#,
        )
        .unwrap();
        assert!(parse(&payload).is_err());
    }

    #[test]
    fn handles_missing_run_time_ticks() {
        let payload: JellyfinPayload = serde_json::from_str(
            r#"{
            "NotificationType": "PlaybackStop",
            "Name": "Track",
            "PlaybackPositionTicks": 100000
        }"#,
        )
        .unwrap();
        let event = parse(&payload).unwrap();
        assert_eq!(event.duration_secs, None);
    }

    #[test]
    fn parse_now_playing_accepts_playback_start() {
        let payload: JellyfinPayload = serde_json::from_str(
            r#"{
            "NotificationType": "PlaybackStart",
            "Artist": "Portishead",
            "Album": "Dummy",
            "Name": "Sour Times",
            "RunTimeTicks": 2460000000
        }"#,
        )
        .unwrap();
        let event = parse_now_playing(&payload).unwrap();
        assert_eq!(event.artist, "Portishead");
        assert_eq!(event.album.as_deref(), Some("Dummy"));
        assert_eq!(event.track, "Sour Times");
        assert_eq!(event.duration_secs, Some(246));
        assert_eq!(event.source, Source::Jellyfin);
    }

    #[test]
    fn parse_now_playing_rejects_non_start_events() {
        let payload: JellyfinPayload = serde_json::from_str(
            r#"{
            "NotificationType": "PlaybackStop",
            "Name": "Track",
            "PlaybackPositionTicks": 100000
        }"#,
        )
        .unwrap();
        assert!(parse_now_playing(&payload).is_err());
    }
}
