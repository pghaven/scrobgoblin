# Now Playing Forwarding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Forward Navidrome's `playing_now` events to ListenBrainz, Last.fm, and optionally Koito so that "now playing" widgets on those services update in real time.

**Architecture:** Add a `NowPlayingEvent` type parallel to `PlayEvent`, parse it from the existing `LbPayload` in the Navidrome handler, and fan it out to per-target `submit_now_playing` functions gated by per-target `forward_now_playing` config flags. No retry logic — now-playing is fire-and-forget.

**Tech Stack:** Rust, axum 0.8, reqwest 0.12, mockito 1 (tests), serde_json, md5 0.7

---

## File Map

| File | Change |
|------|--------|
| `src/event.rs` | Add `NowPlayingEvent` struct |
| `src/config.rs` | Add `forward_now_playing: Option<bool>` to `KoitoConfig`, `ListenBrainzConfig`, `LastFmConfig` |
| `src/sources/navidrome.rs` | Add `parse_now_playing(body: &LbPayload) -> Result<NowPlayingEvent>` |
| `src/targets/listenbrainz.rs` | Add `build_now_playing_payload`, `submit_now_playing_to`, `submit_now_playing` |
| `src/targets/lastfm.rs` | Add `update_now_playing_to`, `update_now_playing` |
| `src/targets/koito.rs` | Add `submit_now_playing_to`, `submit_now_playing` |
| `src/targets/mod.rs` | Add `fan_out_now_playing` |
| `src/router.rs` | Update `navidrome_handler` to call `fan_out_now_playing` instead of silently returning |
| `config.toml.example` | Add `forward_now_playing` options |

---

### Task 1: Add NowPlayingEvent

**Files:**
- Modify: `src/event.rs`

- [ ] **Step 1: Write the failing test**

Add inside the existing `#[cfg(test)]` block at the bottom of `src/event.rs` (there isn't one — add a new one):

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test now_playing_event_source_display
```

Expected: FAIL — `NowPlayingEvent` not defined.

- [ ] **Step 3: Add `NowPlayingEvent` to `src/event.rs`**

Add after the closing `}` of `PlayEvent`:

```rust
#[derive(Debug, Clone)]
pub struct NowPlayingEvent {
    pub artist: String,
    pub album: Option<String>,
    pub track: String,
    pub duration_secs: Option<u64>,
    pub source: Source,
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test now_playing_event_source_display
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/event.rs
git commit -m "feat: add NowPlayingEvent type"
```

---

### Task 2: Add `forward_now_playing` config flags

**Files:**
- Modify: `src/config.rs`
- Modify: `config.toml.example`

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)]` block in `src/config.rs`, after the existing `parses_valid_config` test:

```rust
#[test]
fn parses_forward_now_playing_flags() {
    let toml = r#"
[server]
port = 4567

[koito]
base_url = "http://koito.example.com"
api_key = "koito-key"
forward_now_playing = true

[listenbrainz]
user_token = "lb-token"
forward_now_playing = false

[lastfm]
api_key = "lfm-key"
shared_secret = "lfm-secret"
session_key = "lfm-session"
"#;
    let cfg: Config = toml::from_str(toml).expect("should parse");
    assert_eq!(cfg.koito.forward_now_playing, Some(true));
    assert_eq!(cfg.listenbrainz.forward_now_playing, Some(false));
    assert_eq!(cfg.lastfm.forward_now_playing, None); // omitted → None
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test parses_forward_now_playing_flags
```

Expected: FAIL — `forward_now_playing` field not defined.

- [ ] **Step 3: Add the fields to each config struct**

In `src/config.rs`, update each struct:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct KoitoConfig {
    pub base_url: String,
    pub api_key: String,
    pub forward_now_playing: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListenBrainzConfig {
    pub user_token: String,
    pub forward_now_playing: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LastFmConfig {
    pub api_key: String,
    pub shared_secret: String,
    pub session_key: String,
    pub forward_now_playing: Option<bool>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test
```

Expected: all 24 tests PASS (the existing `parses_valid_config` test still passes because `Option<bool>` fields are absent from that TOML and deserialize to `None`).

- [ ] **Step 5: Update `config.toml.example`**

Replace the `[koito]`, `[listenbrainz]`, and `[lastfm]` sections:

```toml
[koito]
base_url = "http://koito.yourdomain.com"
api_key  = "your-koito-api-key"
# forward_now_playing = false  # default false — enable only after confirming Koito deduplicates

[listenbrainz]
user_token = "your-listenbrainz-token"
# forward_now_playing = true   # default true

[lastfm]
api_key       = "your-lastfm-api-key"
shared_secret = "your-lastfm-shared-secret"
session_key   = "your-lastfm-session-key"
# forward_now_playing = true   # default true
```

- [ ] **Step 6: Commit**

```bash
git add src/config.rs config.toml.example
git commit -m "feat: add forward_now_playing config flags per target"
```

---

### Task 3: Parse now-playing events in Navidrome source

**Files:**
- Modify: `src/sources/navidrome.rs`

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)]` block in `src/sources/navidrome.rs`:

```rust
#[test]
fn parses_now_playing_event() {
    let body: LbPayload = serde_json::from_str(r#"{
        "listen_type": "playing_now",
        "payload": [{
            "track_metadata": {
                "artist_name": "Portishead",
                "track_name": "Glory Box",
                "release_name": "Dummy",
                "additional_info": { "duration": 249 }
            }
        }]
    }"#).unwrap();
    let event = parse_now_playing(&body).unwrap();
    assert_eq!(event.artist, "Portishead");
    assert_eq!(event.track, "Glory Box");
    assert_eq!(event.album.as_deref(), Some("Dummy"));
    assert_eq!(event.duration_secs, Some(249));
    assert_eq!(event.source, crate::event::Source::Navidrome);
}

#[test]
fn now_playing_returns_error_on_empty_payload() {
    let body: LbPayload = serde_json::from_str(r#"{"payload": []}"#).unwrap();
    assert!(parse_now_playing(&body).is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test parses_now_playing_event
```

Expected: FAIL — `parse_now_playing` not defined.

- [ ] **Step 3: Add `parse_now_playing` to `src/sources/navidrome.rs`**

Add the import at the top of the file (after the existing `use crate::event::{PlayEvent, Source};`):

```rust
use crate::event::{NowPlayingEvent, PlayEvent, Source};
```

Add the function after `parse`:

```rust
pub fn parse_now_playing(body: &LbPayload) -> Result<NowPlayingEvent> {
    let listen = body.payload.first().ok_or_else(|| anyhow!("empty payload"))?;
    let meta = &listen.track_metadata;

    let duration_secs = meta.additional_info.as_ref().and_then(|info| {
        info.duration
            .or_else(|| info.duration_ms.map(|ms| ms / 1000))
    });

    Ok(NowPlayingEvent {
        artist: meta.artist_name.clone(),
        album: meta.release_name.clone(),
        track: meta.track_name.clone(),
        duration_secs,
        source: Source::Navidrome,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sources/navidrome.rs src/event.rs
git commit -m "feat: parse playing_now events from Navidrome LB payload"
```

---

### Task 4: ListenBrainz now-playing submit

**Files:**
- Modify: `src/targets/listenbrainz.rs`

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)]` block in `src/targets/listenbrainz.rs`:

```rust
fn test_now_playing_event() -> crate::event::NowPlayingEvent {
    crate::event::NowPlayingEvent {
        artist: "Massive Attack".to_string(),
        album: Some("Mezzanine".to_string()),
        track: "Teardrop".to_string(),
        duration_secs: Some(330),
        source: Source::Navidrome,
    }
}

#[test]
fn now_playing_payload_has_correct_listen_type() {
    let event = test_now_playing_event();
    let payload = build_now_playing_payload(&event);
    assert_eq!(payload["listen_type"], "playing_now");
    assert!(payload["payload"][0]["listened_at"].is_null());
    assert_eq!(payload["payload"][0]["track_metadata"]["artist_name"], "Massive Attack");
    assert_eq!(payload["payload"][0]["track_metadata"]["track_name"], "Teardrop");
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
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test now_playing_payload_has_correct_listen_type
```

Expected: FAIL — `build_now_playing_payload` not defined.

- [ ] **Step 3: Add the functions to `src/targets/listenbrainz.rs`**

Add the import at the top (update the existing `use crate::event::PlayEvent;`):

```rust
use crate::event::{NowPlayingEvent, PlayEvent};
```

Add after `build_lb_payload`:

```rust
pub fn build_now_playing_payload(event: &NowPlayingEvent) -> Value {
    let mut track_metadata = json!({
        "artist_name": event.artist,
        "track_name": event.track,
    });
    if let Some(album) = &event.album {
        track_metadata["release_name"] = json!(album);
    }
    json!({
        "listen_type": "playing_now",
        "payload": [{ "track_metadata": track_metadata }]
    })
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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/targets/listenbrainz.rs
git commit -m "feat: add ListenBrainz now-playing submit"
```

---

### Task 5: Last.fm now-playing update

**Files:**
- Modify: `src/targets/lastfm.rs`

Note: Last.fm's `track.updateNowPlaying` uses param names `artist`, `track`, `album`, `duration` — **without** the `[0]` suffix used in `track.scrobble`. There is no `timestamp` param.

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)]` block in `src/targets/lastfm.rs`:

```rust
fn test_now_playing_event() -> crate::event::NowPlayingEvent {
    crate::event::NowPlayingEvent {
        artist: "LCD Soundsystem".to_string(),
        album: Some("Sound Of Silver".to_string()),
        track: "All My Friends".to_string(),
        duration_secs: Some(447),
        source: crate::event::Source::Jellyfin,
    }
}

#[tokio::test]
async fn update_now_playing_to_posts_correct_method() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/2.0/")
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
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test update_now_playing_to_posts_correct_method
```

Expected: FAIL — `update_now_playing_to` not defined.

- [ ] **Step 3: Add the functions to `src/targets/lastfm.rs`**

Update the import at the top:

```rust
use crate::event::{NowPlayingEvent, PlayEvent};
```

Add after `submit_to`:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/targets/lastfm.rs
git commit -m "feat: add Last.fm track.updateNowPlaying support"
```

---

### Task 6: Koito now-playing submit

**Files:**
- Modify: `src/targets/koito.rs`

Koito's LB-compatible API accepts `playing_now` payloads at the same endpoint as scrobbles. We reuse `build_now_playing_payload` from `listenbrainz.rs`.

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)]` block in `src/targets/koito.rs`:

```rust
fn test_now_playing_event() -> crate::event::NowPlayingEvent {
    crate::event::NowPlayingEvent {
        artist: "Bjork".to_string(),
        album: Some("Homogenic".to_string()),
        track: "Joga".to_string(),
        duration_secs: Some(305),
        source: crate::event::Source::Plex,
    }
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
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test submit_now_playing_to_posts_lb_payload
```

Expected: FAIL — `submit_now_playing_to` (now-playing variant) not defined.

- [ ] **Step 3: Add the functions to `src/targets/koito.rs`**

Update the import at the top:

```rust
use crate::event::{NowPlayingEvent, PlayEvent};
```

Add after `submit_to`:

```rust
pub async fn submit_now_playing(
    cfg: &crate::config::KoitoConfig,
    client: &reqwest::Client,
    event: &NowPlayingEvent,
) -> Result<()> {
    submit_now_playing_to(&cfg.base_url, &cfg.api_key, client, event).await
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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/targets/koito.rs
git commit -m "feat: add Koito now-playing submit"
```

---

### Task 7: fan_out_now_playing

**Files:**
- Modify: `src/targets/mod.rs`

Now-playing is fire-and-forget — no retry loop. Each target is spawned and logged, but failures don't retry (a new `playing_now` event arrives with the next track anyway).

- [ ] **Step 1: Write the failing test**

Add to `src/targets/mod.rs`. Since `fan_out_now_playing` is async and touches real config, the test verifies it compiles and runs without panic on a minimal config:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KoitoConfig, LastFmConfig, ListenBrainzConfig, ServerConfig};
    use crate::event::{NowPlayingEvent, Source};

    fn minimal_cfg() -> Arc<Config> {
        Arc::new(Config {
            server: ServerConfig { port: 4567, webhook_token: None },
            koito: KoitoConfig {
                base_url: "http://localhost:1".to_string(),
                api_key: "k".to_string(),
                forward_now_playing: Some(false),
            },
            listenbrainz: ListenBrainzConfig {
                user_token: "l".to_string(),
                forward_now_playing: Some(false),
            },
            lastfm: LastFmConfig {
                api_key: "a".to_string(),
                shared_secret: "s".to_string(),
                session_key: "k".to_string(),
                forward_now_playing: Some(false),
            },
        })
    }

    #[tokio::test]
    async fn fan_out_now_playing_does_not_panic_when_all_disabled() {
        let cfg = minimal_cfg();
        let client = reqwest::Client::new();
        let event = NowPlayingEvent {
            artist: "Test".to_string(),
            album: None,
            track: "Track".to_string(),
            duration_secs: None,
            source: Source::Navidrome,
        };
        // All targets disabled — should complete immediately without panicking
        fan_out_now_playing(cfg, client, event).await;
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test fan_out_now_playing_does_not_panic
```

Expected: FAIL — `fan_out_now_playing` not defined.

- [ ] **Step 3: Add `fan_out_now_playing` to `src/targets/mod.rs`**

Update the import at the top:

```rust
use crate::{config::Config, event::{NowPlayingEvent, PlayEvent}};
```

Add after `fan_out`:

```rust
pub async fn fan_out_now_playing(cfg: Arc<Config>, client: reqwest::Client, event: NowPlayingEvent) {
    let event = Arc::new(event);

    if cfg.listenbrainz.forward_now_playing.unwrap_or(true) {
        let (cfg1, client1, event1) = (cfg.clone(), client.clone(), event.clone());
        tokio::spawn(async move {
            match listenbrainz::submit_now_playing(&cfg1.listenbrainz, &client1, &event1).await {
                Ok(()) => println!("[NOW] {} → ListenBrainz | {} - {}", event1.source, event1.artist, event1.track),
                Err(e) => eprintln!("[FAIL] {} → ListenBrainz now-playing | {} - {} | {}", event1.source, event1.artist, event1.track, e),
            }
        });
    }

    if cfg.lastfm.forward_now_playing.unwrap_or(true) {
        let (cfg2, client2, event2) = (cfg.clone(), client.clone(), event.clone());
        tokio::spawn(async move {
            match lastfm::update_now_playing(&cfg2.lastfm, &client2, &event2).await {
                Ok(()) => println!("[NOW] {} → Last.fm | {} - {}", event2.source, event2.artist, event2.track),
                Err(e) => eprintln!("[FAIL] {} → Last.fm now-playing | {} - {} | {}", event2.source, event2.artist, event2.track, e),
            }
        });
    }

    if cfg.koito.forward_now_playing.unwrap_or(false) {
        let (cfg3, client3, event3) = (cfg.clone(), client.clone(), event.clone());
        tokio::spawn(async move {
            match koito::submit_now_playing(&cfg3.koito, &client3, &event3).await {
                Ok(()) => println!("[NOW] {} → Koito | {} - {}", event3.source, event3.artist, event3.track),
                Err(e) => eprintln!("[FAIL] {} → Koito now-playing | {} - {} | {}", event3.source, event3.artist, event3.track, e),
            }
        });
    }
}
```

- [ ] **Step 4: Run all tests to verify they pass**

```bash
cargo test
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/targets/mod.rs src/event.rs
git commit -m "feat: add fan_out_now_playing with per-target config gates"
```

---

### Task 8: Wire up router

**Files:**
- Modify: `src/router.rs`

Replace the early `return lb_ok()` for `playing_now` events with a call to `fan_out_now_playing`.

- [ ] **Step 1: Locate the current playing_now guard**

In `src/router.rs`, inside `navidrome_handler`, find:

```rust
if body.listen_type.as_deref() == Some("playing_now") {
    return lb_ok().into_response();
}
```

- [ ] **Step 2: Replace it**

```rust
if body.listen_type.as_deref() == Some("playing_now") {
    match sources::navidrome::parse_now_playing(&body) {
        Ok(event) => { tokio::spawn(targets::fan_out_now_playing(state.cfg, state.client, event)); }
        Err(e) => eprintln!("[WARN] Navidrome now-playing parse error: {}", e),
    }
    return lb_ok().into_response();
}
```

- [ ] **Step 3: Build to verify it compiles**

```bash
cargo build
```

Expected: compiles with no errors.

- [ ] **Step 4: Run all tests**

```bash
cargo test
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/router.rs
git commit -m "feat: forward playing_now events to fan_out_now_playing"
```

---

### Task 9: Update ROADMAP and push

**Files:**
- Modify: `ROADMAP.md`

- [ ] **Step 1: Mark now-playing as done in ROADMAP.md**

Change the now-playing section header and status line:

```markdown
## ✅ Done — Per-target "now playing" forwarding

**Status:** Implemented 2026-05-31.
```

Remove the implementation notes (they're now in the code) and replace with a summary:

```markdown
`playing_now` events from Navidrome are forwarded to ListenBrainz (`track.updateNowPlaying` equivalent via LB API) and Last.fm (`track.updateNowPlaying`). Koito forwarding is off by default (`forward_now_playing = false`) until deduplication behaviour is confirmed. Per-target `forward_now_playing` flags in `config.toml` control which targets receive these events.
```

- [ ] **Step 2: Run tests one final time**

```bash
cargo test
```

Expected: all tests PASS.

- [ ] **Step 3: Commit and push**

```bash
git add ROADMAP.md
git commit -m "docs: mark now-playing forwarding as complete"
git push
```
