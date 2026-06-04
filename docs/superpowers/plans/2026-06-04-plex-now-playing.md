# Plex Now-Playing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fire a now-playing update to all configured scrobble targets when Plex sends a `media.play` or `media.resume` webhook event.

**Architecture:** Add `parse_now_playing` to `src/sources/plex.rs`, then replace the fragile error-message-matching dispatch in `plex_handler` with an explicit match on `plex_payload.event`. The existing `fan_out_now_playing` infrastructure handles delivery unchanged.

**Tech Stack:** Rust, axum, tokio, anyhow. Tests use `tower::ServiceExt::oneshot` and `axum::body::Body`.

---

## Files

- **Modify:** `src/sources/plex.rs` — add `parse_now_playing`, add `NowPlayingEvent` import
- **Modify:** `src/router.rs` — replace tail of `plex_handler` with explicit event-type match; add test helper + two new integration tests

---

### Task 1: Add `parse_now_playing` to `src/sources/plex.rs`

**Files:**
- Modify: `src/sources/plex.rs`

- [ ] **Step 1: Write the failing tests**

Add these three tests inside the existing `#[cfg(test)] mod tests` block at the bottom of `src/sources/plex.rs`:

```rust
#[test]
fn parse_now_playing_accepts_media_play() {
    let payload: PlexPayload = serde_json::from_str(r#"{
        "event": "media.play",
        "Metadata": {
            "grandparentTitle": "Radiohead",
            "parentTitle": "OK Computer",
            "title": "Karma Police",
            "duration": 264000
        }
    }"#).unwrap();
    let event = parse_now_playing(&payload).unwrap();
    assert_eq!(event.artist, "Radiohead");
    assert_eq!(event.album.as_deref(), Some("OK Computer"));
    assert_eq!(event.track, "Karma Police");
    assert_eq!(event.duration_secs, Some(264));
    assert_eq!(event.source, Source::Plex);
}

#[test]
fn parse_now_playing_accepts_media_resume() {
    let payload: PlexPayload = serde_json::from_str(r#"{
        "event": "media.resume",
        "Metadata": {
            "grandparentTitle": "Portishead",
            "title": "Glory Box"
        }
    }"#).unwrap();
    let event = parse_now_playing(&payload).unwrap();
    assert_eq!(event.artist, "Portishead");
    assert_eq!(event.track, "Glory Box");
    assert_eq!(event.album, None);
    assert_eq!(event.duration_secs, None);
}

#[test]
fn parse_now_playing_rejects_non_play_events() {
    let payload: PlexPayload = serde_json::from_str(r#"{
        "event": "media.stop",
        "Metadata": { "title": "Track", "grandparentTitle": "Artist" }
    }"#).unwrap();
    assert!(parse_now_playing(&payload).is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test parse_now_playing
```

Expected: compile error — `parse_now_playing` is not defined yet.

- [ ] **Step 3: Add `NowPlayingEvent` to the import and implement `parse_now_playing`**

Replace the first line of `src/sources/plex.rs`:

```rust
use crate::event::{NowPlayingEvent, PlayEvent, Source};
```

Then add the function after the existing `parse` function (before `#[cfg(test)]`):

```rust
pub fn parse_now_playing(payload: &PlexPayload) -> Result<NowPlayingEvent> {
    if payload.event != "media.play" && payload.event != "media.resume" {
        return Err(anyhow!("not a now-playing event: {}", payload.event));
    }
    let meta = payload
        .metadata
        .as_ref()
        .ok_or_else(|| anyhow!("missing Metadata"))?;

    Ok(NowPlayingEvent {
        artist: meta.grandparent_title.clone().unwrap_or_default(),
        album: meta.parent_title.clone(),
        track: meta.title.clone(),
        duration_secs: meta.duration.map(|ms| ms / 1000),
        source: Source::Plex,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test parse_now_playing
```

Expected: 3 tests pass.

- [ ] **Step 5: Run the full test suite to check for regressions**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/sources/plex.rs
git commit -m "feat: add parse_now_playing to plex source"
```

---

### Task 2: Update `plex_handler` to dispatch now-playing events

**Files:**
- Modify: `src/router.rs`

- [ ] **Step 1: Write the failing tests**

Add a new test helper and two integration tests inside the existing `#[cfg(test)] mod tests` block in `src/router.rs`, after the existing `test_app_plex_token` tests:

```rust
fn test_app_plex_nowplaying() -> Router {
    // All forward_now_playing flags set to false so fan_out_now_playing
    // does not make real HTTP calls during tests.
    let cfg = Arc::new(Config {
        server: crate::config::ServerConfig { port: 4567, webhook_token: None },
        plex: crate::config::PlexConfig { webhook_token: None },
        jellyfin: crate::config::JellyfinConfig { webhook_token: None },
        koito: crate::config::KoitoConfig {
            base_url: "http://k".into(),
            api_key: "k".into(),
            forward_now_playing: Some(false),
        },
        listenbrainz: crate::config::ListenBrainzConfig {
            user_token: "t".into(),
            forward_now_playing: Some(false),
        },
        lastfm: crate::config::LastFmConfig {
            api_key: "a".into(),
            shared_secret: "s".into(),
            session_key: "k".into(),
            forward_now_playing: Some(false),
        },
    });
    build_router(AppState { cfg, client: reqwest::Client::new() })
}

fn plex_nowplaying_request(event_type: &str) -> http::Request<Body> {
    let json = format!(
        r#"{{"event":"{}","Metadata":{{"grandparentTitle":"Radiohead","parentTitle":"OK Computer","title":"Karma Police","duration":264000}}}}"#,
        event_type
    );
    let body = format!(
        "--testboundary\r\nContent-Disposition: form-data; name=\"payload\"\r\n\r\n{}\r\n--testboundary--",
        json
    );
    http::Request::builder()
        .method("POST")
        .uri("/webhooks/plex/open")
        .header("content-type", "multipart/form-data; boundary=testboundary")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn plex_handler_returns_200_for_media_play() {
    let app = test_app_plex_nowplaying();
    let response = app.oneshot(plex_nowplaying_request("media.play")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn plex_handler_returns_200_for_media_resume() {
    let app = test_app_plex_nowplaying();
    let response = app.oneshot(plex_nowplaying_request("media.resume")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn plex_handler_returns_200_for_unrecognised_event() {
    let app = test_app_plex_nowplaying();
    let response = app.oneshot(plex_nowplaying_request("media.stop")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test plex_handler_returns_200_for_media_play
cargo test plex_handler_returns_200_for_media_resume
cargo test plex_handler_returns_200_for_unrecognised_event
```

Expected: the `media.play` and `media.resume` tests pass (handler currently returns 200 for all non-scrobble events via the catch-all), but this confirms the test infrastructure works before we refactor.

- [ ] **Step 3: Replace the dispatch tail of `plex_handler`**

In `src/router.rs`, find the end of `plex_handler` — the `match sources::plex::parse(&plex_payload)` block — and replace it entirely:

Old code (the final match block in `plex_handler`):
```rust
    match sources::plex::parse(&plex_payload) {
        Ok(event) if threshold::qualifies(&event) => {
            tokio::spawn(targets::fan_out(state.cfg, state.client, event));
            StatusCode::OK
        }
        Ok(_) => StatusCode::OK,
        Err(e) if e.to_string().contains("not a scrobble event") => StatusCode::OK,
        Err(e) => {
            eprintln!("[WARN] Plex parse error: {}", e);
            StatusCode::BAD_REQUEST
        }
    }
```

New code:
```rust
    match plex_payload.event.as_str() {
        "media.play" | "media.resume" => {
            match sources::plex::parse_now_playing(&plex_payload) {
                Ok(event) => {
                    println!("[REQ] playing_now (plex) | {} - {}", event.artist, event.track);
                    tokio::spawn(targets::fan_out_now_playing(state.cfg, state.client, event));
                }
                Err(e) => eprintln!("[WARN] Plex now-playing parse error: {}", e),
            }
            StatusCode::OK
        }
        "media.scrobble" => {
            match sources::plex::parse(&plex_payload) {
                Ok(event) if threshold::qualifies(&event) => {
                    tokio::spawn(targets::fan_out(state.cfg, state.client, event));
                }
                Ok(_) => {}
                Err(e) => eprintln!("[WARN] Plex scrobble parse error: {}", e),
            }
            StatusCode::OK
        }
        _ => StatusCode::OK,
    }
```

- [ ] **Step 4: Run all tests**

```bash
cargo test
```

Expected: all tests pass, including the three new ones.

- [ ] **Step 5: Commit**

```bash
git add src/router.rs
git commit -m "feat: dispatch now-playing for Plex media.play and media.resume events"
```
