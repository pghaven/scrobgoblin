use crate::config::LastFmConfig;
use crate::event::{NowPlayingEvent, PlayEvent};
use anyhow::Result;
use std::collections::BTreeMap;

const LFM_BASE_URL: &str = "https://ws.audioscrobbler.com";

pub fn build_signature(params: &BTreeMap<String, String>, shared_secret: &str) -> String {
    let mut sig_str = String::new();
    for (k, v) in params {
        sig_str.push_str(k);
        sig_str.push_str(v);
    }
    sig_str.push_str(shared_secret);
    format!("{:x}", md5::compute(sig_str.as_bytes()))
}

pub async fn submit(
    cfg: &LastFmConfig,
    client: &reqwest::Client,
    event: &PlayEvent,
) -> Result<()> {
    submit_to(LFM_BASE_URL, cfg, client, event).await
}

pub async fn submit_to(
    base_url: &str,
    cfg: &LastFmConfig,
    client: &reqwest::Client,
    event: &PlayEvent,
) -> Result<()> {
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    params.insert("method".to_string(), "track.scrobble".to_string());
    params.insert("api_key".to_string(), cfg.api_key.clone());
    params.insert("sk".to_string(), cfg.session_key.clone());
    params.insert("artist[0]".to_string(), event.artist.clone());
    params.insert("track[0]".to_string(), event.track.clone());
    params.insert("timestamp[0]".to_string(), event.played_at.timestamp().to_string());
    if let Some(album) = &event.album {
        params.insert("album[0]".to_string(), album.clone());
    }
    if let Some(duration) = event.duration_secs {
        params.insert("duration[0]".to_string(), duration.to_string());
    }

    let api_sig = build_signature(&params, &cfg.shared_secret);
    params.insert("api_sig".to_string(), api_sig);
    params.insert("format".to_string(), "json".to_string());

    let resp = client
        .post(format!("{}/2.0/", base_url))
        .form(&params)
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Last.fm error: {}", text);
    }
    Ok(())
}

pub async fn update_now_playing(
    cfg: &LastFmConfig,
    client: &reqwest::Client,
    event: &NowPlayingEvent,
) -> Result<()> {
    update_now_playing_to(LFM_BASE_URL, cfg, client, event).await
}

pub async fn update_now_playing_to(
    base_url: &str,
    cfg: &LastFmConfig,
    client: &reqwest::Client,
    event: &NowPlayingEvent,
) -> Result<()> {
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    params.insert("method".to_string(), "track.updateNowPlaying".to_string());
    params.insert("api_key".to_string(), cfg.api_key.clone());
    params.insert("sk".to_string(), cfg.session_key.clone());
    params.insert("artist".to_string(), event.artist.clone());
    params.insert("track".to_string(), event.track.clone());
    if let Some(album) = &event.album {
        params.insert("album".to_string(), album.clone());
    }
    if let Some(duration) = event.duration_secs {
        params.insert("duration".to_string(), duration.to_string());
    }

    let api_sig = build_signature(&params, &cfg.shared_secret);
    params.insert("api_sig".to_string(), api_sig);
    params.insert("format".to_string(), "json".to_string());

    let resp = client
        .post(format!("{}/2.0/", base_url))
        .form(&params)
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Last.fm error: {}", text);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{PlayEvent, Source};
    use chrono::{TimeZone, Utc};

    fn test_cfg() -> LastFmConfig {
        LastFmConfig {
            api_key: "myapikey".to_string(),
            shared_secret: "mysecret".to_string(),
            session_key: "mysession".to_string(),
            forward_now_playing: None,
        }
    }

    fn test_event() -> PlayEvent {
        PlayEvent {
            artist: "LCD Soundsystem".to_string(),
            album: Some("Sound Of Silver".to_string()),
            track: "All My Friends".to_string(),
            duration_secs: Some(447),
            played_at: Utc.timestamp_opt(1700000000, 0).unwrap(),
            source: Source::Jellyfin,
        }
    }

    fn test_now_playing_event() -> crate::event::NowPlayingEvent {
        crate::event::NowPlayingEvent {
            artist: "LCD Soundsystem".to_string(),
            album: Some("Sound Of Silver".to_string()),
            track: "All My Friends".to_string(),
            duration_secs: Some(447),
            source: crate::event::Source::Jellyfin,
        }
    }

    #[test]
    fn signature_is_deterministic() {
        let mut params = BTreeMap::new();
        params.insert("api_key".to_string(), "key".to_string());
        params.insert("method".to_string(), "track.scrobble".to_string());
        let sig1 = build_signature(&params, "secret");
        let sig2 = build_signature(&params, "secret");
        assert_eq!(sig1, sig2);
        assert_eq!(sig1.len(), 32); // MD5 hex
    }

    #[test]
    fn signature_changes_with_different_secret() {
        let mut params = BTreeMap::new();
        params.insert("method".to_string(), "track.scrobble".to_string());
        let sig1 = build_signature(&params, "secret1");
        let sig2 = build_signature(&params, "secret2");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn signature_uses_alphabetical_param_order() {
        let mut params = BTreeMap::new();
        params.insert("b_param".to_string(), "B".to_string());
        params.insert("a_param".to_string(), "A".to_string());
        let sig = build_signature(&params, "s");
        // BTreeMap iterates alphabetically: a_param then b_param
        // Concatenation: a_paramAb_paramBs
        let expected = format!("{:x}", md5::compute(b"a_paramAb_paramBs"));
        assert_eq!(sig, expected);
    }

    #[tokio::test]
    async fn submit_to_posts_form_with_signature() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/2.0/")
            .with_status(200)
            .with_body(r#"{"scrobbles":{"@attr":{"accepted":1}}}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let cfg = test_cfg();
        let event = test_event();
        let result = submit_to(&server.url(), &cfg, &client, &event).await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn submit_to_returns_error_on_non_200() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/2.0/")
            .with_status(403)
            .with_body("forbidden")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let cfg = test_cfg();
        let event = test_event();
        let result = submit_to(&server.url(), &cfg, &client, &event).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn update_now_playing_to_posts_correct_method() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/2.0/")
            .match_body(mockito::Matcher::Regex("method=track.updateNowPlaying".to_string()))
            .with_status(200)
            .with_body(r#"{"nowplaying":{"artist":{"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let cfg = test_cfg();
        let event = test_now_playing_event();
        let result = update_now_playing_to(&server.url(), &cfg, &client, &event).await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn update_now_playing_to_returns_error_on_non_200() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/2.0/")
            .with_status(403)
            .with_body("forbidden")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let cfg = test_cfg();
        let event = test_now_playing_event();
        let result = update_now_playing_to(&server.url(), &cfg, &client, &event).await;
        assert!(result.is_err());
    }
}
