use crate::event::{PlayEvent, Source};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct LbPayload {
    pub listen_type: Option<String>,
    pub payload: Vec<LbListen>,
}

#[derive(Debug, Deserialize)]
pub struct LbListen {
    pub listened_at: Option<i64>,
    pub track_metadata: LbTrackMetadata,
}

#[derive(Debug, Deserialize)]
pub struct LbTrackMetadata {
    pub artist_name: String,
    pub track_name: String,
    pub release_name: Option<String>,
    pub additional_info: Option<LbAdditionalInfo>,
}

#[derive(Debug, Deserialize)]
pub struct LbAdditionalInfo {
    pub duration: Option<u64>,
    pub duration_ms: Option<u64>,
}

pub fn parse(body: &LbPayload) -> Result<PlayEvent> {
    let listen = body.payload.first().ok_or_else(|| anyhow!("empty payload"))?;
    let meta = &listen.track_metadata;

    let played_at = listen
        .listened_at
        .and_then(|ts| DateTime::from_timestamp(ts, 0))
        .unwrap_or_else(Utc::now);

    let duration_secs = meta.additional_info.as_ref().and_then(|info| {
        info.duration
            .or_else(|| info.duration_ms.map(|ms| ms / 1000))
    });

    Ok(PlayEvent {
        artist: meta.artist_name.clone(),
        album: meta.release_name.clone(),
        track: meta.track_name.clone(),
        duration_secs,
        played_at,
        source: Source::Navidrome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> LbPayload {
        serde_json::from_str(r#"{
            "listen_type": "single",
            "payload": [{
                "listened_at": 1700000000,
                "track_metadata": {
                    "artist_name": "The Beatles",
                    "track_name": "Hey Jude",
                    "release_name": "Hey Jude Single",
                    "additional_info": { "duration": 431 }
                }
            }]
        }"#).unwrap()
    }

    #[test]
    fn parses_full_payload() {
        let event = parse(&sample_payload()).unwrap();
        assert_eq!(event.artist, "The Beatles");
        assert_eq!(event.track, "Hey Jude");
        assert_eq!(event.album.as_deref(), Some("Hey Jude Single"));
        assert_eq!(event.duration_secs, Some(431));
        assert_eq!(event.source, Source::Navidrome);
    }

    #[test]
    fn parses_payload_without_additional_info() {
        let body: LbPayload = serde_json::from_str(r#"{
            "listen_type": "single",
            "payload": [{
                "track_metadata": {
                    "artist_name": "Artist",
                    "track_name": "Track"
                }
            }]
        }"#).unwrap();
        let event = parse(&body).unwrap();
        assert_eq!(event.duration_secs, None);
        assert_eq!(event.album, None);
    }

    #[test]
    fn returns_error_on_empty_payload() {
        let body: LbPayload = serde_json::from_str(r#"{"payload": []}"#).unwrap();
        assert!(parse(&body).is_err());
    }

    #[test]
    fn captures_playing_now_listen_type() {
        let body: LbPayload = serde_json::from_str(r#"{
            "listen_type": "playing_now",
            "payload": [{
                "track_metadata": {
                    "artist_name": "Artist",
                    "track_name": "Track"
                }
            }]
        }"#).unwrap();
        assert_eq!(body.listen_type.as_deref(), Some("playing_now"));
    }
}
