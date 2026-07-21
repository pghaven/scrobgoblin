use crate::event::{NowPlayingEvent, PlayEvent};
use anyhow::Result;
use serde_json::{json, Value};

const LB_BASE_URL: &str = "https://api.listenbrainz.org";

use crate::targets::ScrobbleTarget;
use async_trait::async_trait;

pub struct ListenBrainzTarget {
    cfg: crate::config::ListenBrainzConfig,
    client: reqwest::Client,
}

impl ListenBrainzTarget {
    pub fn from_config(cfg: &crate::config::ListenBrainzConfig, client: reqwest::Client) -> Self {
        Self { cfg: cfg.clone(), client }
    }
}

#[async_trait]
impl ScrobbleTarget for ListenBrainzTarget {
    fn name(&self) -> &'static str {
        "ListenBrainz"
    }

    async fn submit(&self, event: &PlayEvent) -> Result<()> {
        submit_to(LB_BASE_URL, &self.cfg.user_token, &self.client, event).await
    }

    async fn submit_now_playing(&self, event: &NowPlayingEvent) -> Result<()> {
        if !self.cfg.forward_now_playing.unwrap_or(true) {
            return Ok(());
        }
        submit_now_playing_to(LB_BASE_URL, &self.cfg.user_token, &self.client, event).await
    }
}

pub fn build_lb_payload(event: &PlayEvent) -> Value {
    let mut track_metadata = json!({
        "artist_name": event.artist,
        "track_name": event.track,
    });
    if let Some(album) = &event.album {
        track_metadata["release_name"] = json!(album);
    }
    json!({
        "listen_type": "single",
        "payload": [{
            "listened_at": event.played_at.timestamp(),
            "track_metadata": track_metadata
        }]
    })
}

pub fn build_now_playing_payload(event: &NowPlayingEvent) -> Value {
    let mut track_metadata = json!({
        "artist_name": event.artist,
        "track_name": event.track,
    });
    if let Some(album) = &event.album {
        track_metadata["release_name"] = json!(album);
    }
    if let Some(duration) = event.duration_secs {
        track_metadata["additional_info"] = json!({ "duration": duration });
    }
    json!({
        "listen_type": "playing_now",
        "payload": [{ "track_metadata": track_metadata }]
    })
}

pub async fn submit(
    cfg: &crate::config::ListenBrainzConfig,
    client: &reqwest::Client,
    event: &PlayEvent,
) -> Result<()> {
    submit_to(LB_BASE_URL, &cfg.user_token, client, event).await
}

pub async fn submit_to(
    base_url: &str,
    token: &str,
    client: &reqwest::Client,
    event: &PlayEvent,
) -> Result<()> {
    let body = build_lb_payload(event);
    let resp = client
        .post(format!("{}/1/submit-listens", base_url))
        .header("Authorization", format!("Token {}", token))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("ListenBrainz error: {}", text);
    }
    Ok(())
}

pub async fn submit_now_playing_to(
    base_url: &str,
    token: &str,
    client: &reqwest::Client,
    event: &NowPlayingEvent,
) -> Result<()> {
    let body = build_now_playing_payload(event);
    let resp = client
        .post(format!("{}/1/submit-listens", base_url))
        .header("Authorization", format!("Token {}", token))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("ListenBrainz error: {}", text);
    }
    Ok(())
}

pub async fn submit_now_playing(
    cfg: &crate::config::ListenBrainzConfig,
    client: &reqwest::Client,
    event: &NowPlayingEvent,
) -> Result<()> {
    submit_now_playing_to(LB_BASE_URL, &cfg.user_token, client, event).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{NowPlayingEvent, PlayEvent, Source};
    use chrono::{TimeZone, Utc};

    fn test_event() -> PlayEvent {
        PlayEvent {
            artist: "Massive Attack".to_string(),
            album: Some("Mezzanine".to_string()),
            track: "Teardrop".to_string(),
            duration_secs: Some(330),
            played_at: Utc.timestamp_opt(1700000000, 0).unwrap(),
            source: Source::Navidrome,
        }
    }

    fn test_now_playing_event() -> NowPlayingEvent {
        NowPlayingEvent {
            artist: "Massive Attack".to_string(),
            album: Some("Mezzanine".to_string()),
            track: "Teardrop".to_string(),
            duration_secs: Some(330),
            source: Source::Navidrome,
        }
    }

    #[test]
    fn payload_has_required_fields() {
        let event = test_event();
        let payload = build_lb_payload(&event);
        assert_eq!(payload["listen_type"], "single");
        assert_eq!(payload["payload"][0]["listened_at"], 1700000000i64);
        assert_eq!(payload["payload"][0]["track_metadata"]["artist_name"], "Massive Attack");
        assert_eq!(payload["payload"][0]["track_metadata"]["track_name"], "Teardrop");
        assert_eq!(payload["payload"][0]["track_metadata"]["release_name"], "Mezzanine");
    }

    #[test]
    fn payload_omits_missing_album() {
        let mut event = test_event();
        event.album = None;
        let payload = build_lb_payload(&event);
        assert!(payload["payload"][0]["track_metadata"]["release_name"].is_null());
    }

    #[tokio::test]
    async fn submit_to_sends_correct_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/1/submit-listens")
            .match_header("authorization", "Token test-token")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let event = test_event();
        let result = submit_to(&server.url(), "test-token", &client, &event).await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn submit_to_returns_error_on_failure() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/1/submit-listens")
            .with_status(500)
            .with_body("server error")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let event = test_event();
        let result = submit_to(&server.url(), "test-token", &client, &event).await;
        assert!(result.is_err());
    }

    #[test]
    fn now_playing_payload_has_correct_listen_type() {
        let event = test_now_playing_event();
        let payload = build_now_playing_payload(&event);
        assert_eq!(payload["listen_type"], "playing_now");
        assert!(payload["payload"][0]["listened_at"].is_null());
        assert_eq!(payload["payload"][0]["track_metadata"]["artist_name"], "Massive Attack");
        assert_eq!(payload["payload"][0]["track_metadata"]["track_name"], "Teardrop");
        assert_eq!(payload["payload"][0]["track_metadata"]["additional_info"]["duration"], 330u64);
    }

    #[tokio::test]
    async fn submit_now_playing_to_sends_correct_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/1/submit-listens")
            .match_header("authorization", "Token test-token")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let event = test_now_playing_event();
        let result = submit_now_playing_to(&server.url(), "test-token", &client, &event).await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }

    use crate::targets::ScrobbleTarget;

    fn test_lb_config(forward_now_playing: Option<bool>) -> crate::config::ListenBrainzConfig {
        crate::config::ListenBrainzConfig {
            user_token: "test-token".to_string(),
            forward_now_playing,
        }
    }

    #[test]
    fn listenbrainz_target_name_is_listenbrainz() {
        let target = ListenBrainzTarget::from_config(&test_lb_config(None), reqwest::Client::new());
        assert_eq!(target.name(), "ListenBrainz");
    }

    #[test]
    fn listenbrainz_forward_now_playing_defaults_to_true_when_unset() {
        let cfg = test_lb_config(None);
        assert!(cfg.forward_now_playing.unwrap_or(true));
    }

    #[test]
    fn listenbrainz_forward_now_playing_respects_explicit_false() {
        let cfg = test_lb_config(Some(false));
        assert!(!cfg.forward_now_playing.unwrap_or(true));
    }
}
