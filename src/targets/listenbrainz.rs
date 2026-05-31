use crate::event::PlayEvent;
use anyhow::Result;
use serde_json::{json, Value};

const LB_BASE_URL: &str = "https://api.listenbrainz.org";

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{PlayEvent, Source};
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
}
