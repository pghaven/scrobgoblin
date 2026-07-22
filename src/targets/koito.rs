use crate::event::{NowPlayingEvent, PlayEvent};
use crate::targets::ScrobbleTarget;
use anyhow::Result;
use async_trait::async_trait;

pub struct KoitoTarget {
    cfg: crate::config::KoitoConfig,
    client: reqwest::Client,
}

impl KoitoTarget {
    pub fn from_config(cfg: &crate::config::KoitoConfig, client: reqwest::Client) -> Self {
        Self {
            cfg: cfg.clone(),
            client,
        }
    }
}

#[async_trait]
impl ScrobbleTarget for KoitoTarget {
    fn name(&self) -> &'static str {
        "Koito"
    }

    async fn submit(&self, event: &PlayEvent) -> Result<()> {
        submit_to(&self.cfg.base_url, &self.cfg.api_key, &self.client, event).await
    }

    async fn submit_now_playing(&self, event: &NowPlayingEvent) -> Result<()> {
        if !self.cfg.forward_now_playing.unwrap_or(false) {
            return Ok(());
        }
        submit_now_playing_to(&self.cfg.base_url, &self.cfg.api_key, &self.client, event).await
    }
}

pub async fn submit_to(
    base_url: &str,
    api_key: &str,
    client: &reqwest::Client,
    event: &PlayEvent,
) -> Result<()> {
    let body = crate::targets::listenbrainz::build_lb_payload(event);
    let resp = client
        .post(format!("{}/apis/listenbrainz/1/submit-listens", base_url))
        .header("Authorization", format!("Token {}", api_key))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Koito HTTP {} | {}", status, text);
    }
    Ok(())
}

pub async fn submit_now_playing_to(
    base_url: &str,
    api_key: &str,
    client: &reqwest::Client,
    event: &NowPlayingEvent,
) -> Result<()> {
    let body = crate::targets::listenbrainz::build_now_playing_payload(event);
    let resp = client
        .post(format!("{}/apis/listenbrainz/1/submit-listens", base_url))
        .header("Authorization", format!("Token {}", api_key))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Koito HTTP {} | {}", status, text);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{PlayEvent, Source};
    use crate::targets::ScrobbleTarget;
    use chrono::{TimeZone, Utc};

    fn test_event() -> PlayEvent {
        PlayEvent {
            artist: "Bjork".to_string(),
            album: Some("Homogenic".to_string()),
            track: "Joga".to_string(),
            duration_secs: Some(305),
            played_at: Utc.timestamp_opt(1700000000, 0).unwrap(),
            source: Source::Plex,
        }
    }

    fn test_now_playing_event() -> NowPlayingEvent {
        NowPlayingEvent {
            artist: "Bjork".to_string(),
            album: Some("Homogenic".to_string()),
            track: "Joga".to_string(),
            duration_secs: Some(305),
            source: Source::Plex,
        }
    }

    fn test_koito_config(forward_now_playing: Option<bool>) -> crate::config::KoitoConfig {
        crate::config::KoitoConfig {
            base_url: "http://placeholder".to_string(),
            api_key: "koito-key".to_string(),
            forward_now_playing,
        }
    }

    #[tokio::test]
    async fn submit_to_posts_lb_payload() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/apis/listenbrainz/1/submit-listens")
            .match_header("authorization", "Token koito-key")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let event = test_event();
        let result = submit_to(&server.url(), "koito-key", &client, &event).await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn submit_to_returns_error_on_non_200() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/apis/listenbrainz/1/submit-listens")
            .with_status(401)
            .with_body("unauthorized")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let event = test_event();
        let result = submit_to(&server.url(), "bad-key", &client, &event).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn submit_now_playing_to_posts_lb_payload() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/apis/listenbrainz/1/submit-listens")
            .match_header("authorization", "Token koito-key")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let event = test_now_playing_event();
        let result = submit_now_playing_to(&server.url(), "koito-key", &client, &event).await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn submit_now_playing_to_returns_error_on_non_200() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/apis/listenbrainz/1/submit-listens")
            .with_status(401)
            .with_body("unauthorized")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let event = test_now_playing_event();
        let result = submit_now_playing_to(&server.url(), "bad-key", &client, &event).await;
        assert!(result.is_err());
    }

    #[test]
    fn koito_target_name_is_koito() {
        let target = KoitoTarget::from_config(&test_koito_config(None), reqwest::Client::new());
        assert_eq!(target.name(), "Koito");
    }

    #[tokio::test]
    async fn koito_target_submit_posts_lb_payload() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/apis/listenbrainz/1/submit-listens")
            .match_header("authorization", "Token koito-key")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create_async()
            .await;

        let mut cfg = test_koito_config(None);
        cfg.base_url = server.url();
        let target = KoitoTarget::from_config(&cfg, reqwest::Client::new());
        let result = target.submit(&test_event()).await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn koito_target_submit_now_playing_defaults_off() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/apis/listenbrainz/1/submit-listens")
            .expect(0)
            .create_async()
            .await;

        let mut cfg = test_koito_config(None); // forward_now_playing not set -> defaults false
        cfg.base_url = server.url();
        let target = KoitoTarget::from_config(&cfg, reqwest::Client::new());
        let result = target.submit_now_playing(&test_now_playing_event()).await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn koito_target_submit_now_playing_sends_when_enabled() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/apis/listenbrainz/1/submit-listens")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create_async()
            .await;

        let mut cfg = test_koito_config(Some(true));
        cfg.base_url = server.url();
        let target = KoitoTarget::from_config(&cfg, reqwest::Client::new());
        let result = target.submit_now_playing(&test_now_playing_event()).await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }
}
