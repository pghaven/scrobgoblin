# Scrobble Target Trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make each scrobble target (Koito, ListenBrainz, Last.fm) an independently configurable, self-contained object behind a shared `ScrobbleTarget` trait, so any subset of targets can be configured and new targets can be added without modifying shared fan-out logic.

**Architecture:** Introduce a `ScrobbleTarget` async trait (`name`, `submit`, `submit_now_playing` with a no-op default, `submit_with_retry` with a shared default retry policy). Each target becomes a struct owning its config + a `reqwest::Client`, constructed via `from_config`. `build_targets(&Config, Client) -> Vec<Arc<dyn ScrobbleTarget>>` constructs only the targets whose config section is present. `fan_out`/`fan_out_now_playing` become thin wrappers around a shared `dispatch()` helper that loops over the `Vec`, replacing the three hardcoded per-target spawns.

**Tech Stack:** Rust, axum, tokio, reqwest, `async-trait` (new dependency), mockito (tests).

## Global Constraints

- Design spec: `docs/superpowers/specs/2026-07-21-scrobble-target-trait-design.md` — follow it exactly; this plan implements it task-by-task.
- Scope is targets only (Koito, ListenBrainz, Last.fm). Source-side flexibility (Navidrome/Plex/Jellyfin) is out of scope — tracked in Forgejo issue #10.
- `fan_out` must keep joining all spawned target tasks (same as today); `fan_out_now_playing` must remain fire-and-forget (not joined) — this is an explicit, confirmed design decision, not an oversight.
- `submit_now_playing` gets **no retry** — single attempt only, matching current behavior.
- Retry policy for `submit` (scrobbles): 3 attempts, 1s → 4s backoff — unchanged from today, just centralized in the trait's default `submit_with_retry` method.
- Run `cargo test` after every task and confirm all tests pass before moving to the next task.

---

### Task 1: Add `async-trait` dependency and define the `ScrobbleTarget` trait

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/targets/mod.rs`
- Test: `src/targets/mod.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub trait ScrobbleTarget: Send + Sync` with `fn name(&self) -> &'static str`, `async fn submit(&self, event: &PlayEvent) -> anyhow::Result<()>`, `async fn submit_now_playing(&self, event: &NowPlayingEvent) -> anyhow::Result<()>` (default: `Ok(())`), `async fn submit_with_retry(&self, event: &PlayEvent)` (default: calls `self.submit`, 3 attempts, 1s/4s backoff, using the existing `retry_log` helper).
- This task is additive — the existing `fan_out`/`fan_out_now_playing` functions and their call sites in `router.rs`/`main.rs` are untouched and must keep compiling and passing exactly as before.

- [ ] **Step 1: Add the `async-trait` dependency**

In `Cargo.toml`, add to `[dependencies]` (alphabetical position, after `anyhow`):

```toml
anyhow = "1"
async-trait = "0.1"
axum = { version = "0.8", features = ["multipart"] }
```

- [ ] **Step 2: Write the failing tests for the trait's default behavior**

In `src/targets/mod.rs`, inside the existing `#[cfg(test)] mod tests` block, add (alongside the existing `use super::*;` and other imports already there):

```rust
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingTarget {
    submit_calls: Arc<AtomicUsize>,
    submit_now_playing_calls: Arc<AtomicUsize>,
    fail_submit: bool,
}

#[async_trait]
impl ScrobbleTarget for CountingTarget {
    fn name(&self) -> &'static str {
        "Counting"
    }

    async fn submit(&self, _event: &PlayEvent) -> anyhow::Result<()> {
        self.submit_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_submit {
            anyhow::bail!("forced failure");
        }
        Ok(())
    }

    async fn submit_now_playing(&self, _event: &NowPlayingEvent) -> anyhow::Result<()> {
        self.submit_now_playing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct DefaultOnlyTarget;

#[async_trait]
impl ScrobbleTarget for DefaultOnlyTarget {
    fn name(&self) -> &'static str {
        "DefaultOnly"
    }

    async fn submit(&self, _event: &PlayEvent) -> anyhow::Result<()> {
        Ok(())
    }
    // submit_now_playing not overridden — uses the trait's default no-op
}

fn test_play_event() -> PlayEvent {
    PlayEvent {
        artist: "Test Artist".to_string(),
        album: None,
        track: "Test Track".to_string(),
        duration_secs: Some(200),
        played_at: chrono::Utc::now(),
        source: crate::event::Source::Navidrome,
    }
}

#[tokio::test]
async fn default_submit_now_playing_returns_ok() {
    let target = DefaultOnlyTarget;
    let event = NowPlayingEvent {
        artist: "Test".to_string(),
        album: None,
        track: "Track".to_string(),
        duration_secs: None,
        source: crate::event::Source::Navidrome,
    };
    let result = target.submit_now_playing(&event).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn submit_with_retry_calls_submit_once_on_immediate_success() {
    let calls = Arc::new(AtomicUsize::new(0));
    let target = CountingTarget {
        submit_calls: calls.clone(),
        submit_now_playing_calls: Arc::new(AtomicUsize::new(0)),
        fail_submit: false,
    };
    let event = test_play_event();
    target.submit_with_retry(&event).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
```

- [ ] **Step 3: Run tests to verify they fail to compile**

Run: `cargo test --lib targets`
Expected: compile error — `ScrobbleTarget` trait does not exist yet.

- [ ] **Step 4: Define the `ScrobbleTarget` trait**

In `src/targets/mod.rs`, add near the top of the file (after the existing `use` statements, before `pub async fn fan_out`):

```rust
use async_trait::async_trait;

#[async_trait]
pub trait ScrobbleTarget: Send + Sync {
    fn name(&self) -> &'static str;

    async fn submit(&self, event: &PlayEvent) -> anyhow::Result<()>;

    /// Default no-op: targets that don't support now-playing write zero code.
    async fn submit_now_playing(&self, _event: &NowPlayingEvent) -> anyhow::Result<()> {
        Ok(())
    }

    /// Default retry policy: 3 attempts, 1s -> 4s backoff. Individual targets
    /// may override if a specific API needs different retry behavior.
    async fn submit_with_retry(&self, event: &PlayEvent) {
        for attempt in 1..=3u32 {
            match self.submit(event).await {
                Ok(()) => {
                    println!(
                        "[OK] {} → {} | {} - {}",
                        event.source, self.name(), event.artist, event.track
                    );
                    return;
                }
                Err(e) => retry_log(self.name(), event, attempt, &e).await,
            }
        }
    }
}
```

This reuses the existing `retry_log` free function already defined lower in the file — no changes needed to it.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib targets`
Expected: PASS — `default_submit_now_playing_returns_ok` and `submit_with_retry_calls_submit_once_on_immediate_success` both green, plus all pre-existing tests in the file still pass.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test`
Expected: PASS — no regressions in `src/router.rs`, `src/targets/koito.rs`, etc.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/targets/mod.rs
git commit -m "feat: add ScrobbleTarget trait with default retry and now-playing no-op"
```

---

### Task 2: Refactor Koito into a `KoitoTarget` struct implementing `ScrobbleTarget`

**Files:**
- Modify: `src/targets/koito.rs`

**Interfaces:**
- Consumes: `ScrobbleTarget` trait (Task 1), `crate::config::KoitoConfig` (unchanged shape at this point — still non-`Option` on `Config` until Task 5).
- Produces: `pub struct KoitoTarget { cfg: KoitoConfig, client: reqwest::Client }` with `pub fn from_config(cfg: &KoitoConfig, client: reqwest::Client) -> Self`, and an `impl ScrobbleTarget for KoitoTarget` using the existing `submit_to`/`submit_now_playing_to` free functions unchanged.
- This task is additive — existing `koito::submit`/`koito::submit_now_playing` free functions and their tests are untouched.

- [ ] **Step 1: Write the failing tests**

In `src/targets/koito.rs`, inside the existing `#[cfg(test)] mod tests` block, add:

```rust
use crate::targets::ScrobbleTarget;

fn test_koito_config(forward_now_playing: Option<bool>) -> crate::config::KoitoConfig {
    crate::config::KoitoConfig {
        base_url: "http://placeholder".to_string(),
        api_key: "koito-key".to_string(),
        forward_now_playing,
    }
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib targets::koito`
Expected: compile error — `KoitoTarget` does not exist yet.

- [ ] **Step 3: Implement `KoitoTarget`**

In `src/targets/koito.rs`, add (before the existing free functions, after the `use` statements):

```rust
use crate::targets::ScrobbleTarget;
use async_trait::async_trait;

pub struct KoitoTarget {
    cfg: crate::config::KoitoConfig,
    client: reqwest::Client,
}

impl KoitoTarget {
    pub fn from_config(cfg: &crate::config::KoitoConfig, client: reqwest::Client) -> Self {
        Self { cfg: cfg.clone(), client }
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib targets::koito`
Expected: PASS — all 4 new tests plus the 4 pre-existing `submit_to`/`submit_now_playing_to` tests green.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/targets/koito.rs
git commit -m "feat: add KoitoTarget implementing ScrobbleTarget"
```

---

### Task 3: Refactor ListenBrainz into a `ListenBrainzTarget` struct implementing `ScrobbleTarget`

**Files:**
- Modify: `src/targets/listenbrainz.rs`

**Interfaces:**
- Consumes: `ScrobbleTarget` trait (Task 1).
- Produces: `pub struct ListenBrainzTarget { cfg: ListenBrainzConfig, client: reqwest::Client }` with `pub fn from_config(cfg: &ListenBrainzConfig, client: reqwest::Client) -> Self`, and `impl ScrobbleTarget for ListenBrainzTarget` using the existing `submit_to`/`submit_now_playing_to` free functions and the `LB_BASE_URL` constant, unchanged.

- [ ] **Step 1: Write the failing tests**

In `src/targets/listenbrainz.rs`, inside the existing `#[cfg(test)] mod tests` block, add:

```rust
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
```

Note: `ListenBrainzTarget` always posts to the hardcoded `LB_BASE_URL` constant, so its `submit_now_playing` behavior can't be exercised end-to-end via mockito without also refactoring `LB_BASE_URL` into an instance field (out of scope for this task). The two `#[test]` functions above cover the default-resolution logic (the part that changed in this task); the HTTP-request-building logic itself is already covered by the pre-existing `submit_now_playing_to_sends_correct_request` test in this file, which the trait impl delegates to unchanged.

(Remove the `listenbrainz_target_submit_now_playing_defaults_on` test and the nonexistent `ListenBrainzTargetWithBaseUrlOverrideForTest` reference above — they don't compile. Use only the two `#[test]` functions shown in this corrected step.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib targets::listenbrainz`
Expected: compile error — `ListenBrainzTarget` does not exist yet.

- [ ] **Step 3: Implement `ListenBrainzTarget`**

In `src/targets/listenbrainz.rs`, add (after the `LB_BASE_URL` constant, before the existing free functions):

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib targets::listenbrainz`
Expected: PASS.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/targets/listenbrainz.rs
git commit -m "feat: add ListenBrainzTarget implementing ScrobbleTarget"
```

---

### Task 4: Refactor Last.fm into a `LastFmTarget` struct implementing `ScrobbleTarget`

**Files:**
- Modify: `src/targets/lastfm.rs`

**Interfaces:**
- Consumes: `ScrobbleTarget` trait (Task 1).
- Produces: `pub struct LastFmTarget { cfg: LastFmConfig, client: reqwest::Client }` with `pub fn from_config(cfg: &LastFmConfig, client: reqwest::Client) -> Self`, and `impl ScrobbleTarget for LastFmTarget` using the existing `submit_to`/`update_now_playing_to` free functions and `LFM_BASE_URL` constant, unchanged.

- [ ] **Step 1: Write the failing tests**

In `src/targets/lastfm.rs`, inside the existing `#[cfg(test)] mod tests` block, add:

```rust
use crate::targets::ScrobbleTarget;

fn test_target_cfg(forward_now_playing: Option<bool>) -> LastFmConfig {
    LastFmConfig {
        api_key: "myapikey".to_string(),
        shared_secret: "mysecret".to_string(),
        session_key: "mysession".to_string(),
        forward_now_playing,
    }
}

#[test]
fn lastfm_target_name_is_lastfm() {
    let target = LastFmTarget::from_config(&test_target_cfg(None), reqwest::Client::new());
    assert_eq!(target.name(), "Last.fm");
}

#[test]
fn lastfm_forward_now_playing_defaults_to_true_when_unset() {
    let cfg = test_target_cfg(None);
    assert!(cfg.forward_now_playing.unwrap_or(true));
}

#[test]
fn lastfm_forward_now_playing_respects_explicit_false() {
    let cfg = test_target_cfg(Some(false));
    assert!(!cfg.forward_now_playing.unwrap_or(true));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib targets::lastfm`
Expected: compile error — `LastFmTarget` does not exist yet.

- [ ] **Step 3: Implement `LastFmTarget`**

In `src/targets/lastfm.rs`, add (after the `LFM_BASE_URL` constant, before the existing free functions):

```rust
use crate::targets::ScrobbleTarget;
use async_trait::async_trait;

pub struct LastFmTarget {
    cfg: LastFmConfig,
    client: reqwest::Client,
}

impl LastFmTarget {
    pub fn from_config(cfg: &LastFmConfig, client: reqwest::Client) -> Self {
        Self { cfg: cfg.clone(), client }
    }
}

#[async_trait]
impl ScrobbleTarget for LastFmTarget {
    fn name(&self) -> &'static str {
        "Last.fm"
    }

    async fn submit(&self, event: &PlayEvent) -> Result<()> {
        submit_to(LFM_BASE_URL, &self.cfg, &self.client, event).await
    }

    async fn submit_now_playing(&self, event: &NowPlayingEvent) -> Result<()> {
        if !self.cfg.forward_now_playing.unwrap_or(true) {
            return Ok(());
        }
        update_now_playing_to(LFM_BASE_URL, &self.cfg, &self.client, event).await
    }
}
```

Note `LastFmConfig` must be `Clone` for `cfg.clone()` to work — it already derives `Clone` in `src/config.rs`, no change needed there.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib targets::lastfm`
Expected: PASS.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/targets/lastfm.rs
git commit -m "feat: add LastFmTarget implementing ScrobbleTarget"
```

---

### Task 5: Wire target objects through config, fan-out, and router

This task is a single coupled unit: making the target config sections `Option` breaks the existing `fan_out`/`fan_out_now_playing` signatures and their call sites simultaneously, so these changes cannot be split into independently-compiling sub-steps. Apply all edits below, then run the full test suite once at the end.

**Files:**
- Modify: `src/config.rs`
- Modify: `src/targets/mod.rs`
- Modify: `src/router.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `KoitoTarget`, `ListenBrainzTarget`, `LastFmTarget` (Tasks 2-4), `ScrobbleTarget` trait (Task 1).
- Produces: `Config.koito: Option<KoitoConfig>`, `Config.listenbrainz: Option<ListenBrainzConfig>`, `Config.lastfm: Option<LastFmConfig>`. `pub fn build_targets(cfg: &Config, client: reqwest::Client) -> Vec<Arc<dyn ScrobbleTarget>>`. `pub async fn fan_out(targets: Vec<Arc<dyn ScrobbleTarget>>, event: PlayEvent)` and `pub async fn fan_out_now_playing(targets: Vec<Arc<dyn ScrobbleTarget>>, event: NowPlayingEvent)` — **signatures changed** (no longer take `Arc<Config>`/`reqwest::Client`). `AppState { cfg: Arc<Config>, targets: Vec<Arc<dyn targets::ScrobbleTarget>> }` — **`client` field removed** (no longer used directly by `router.rs`; each target already owns its own cloned client).

- [ ] **Step 1: Make target config sections optional in `src/config.rs`**

Change:

```rust
    pub koito: KoitoConfig,
    pub listenbrainz: ListenBrainzConfig,
    pub lastfm: LastFmConfig,
```

to:

```rust
    #[serde(default)]
    pub koito: Option<KoitoConfig>,
    #[serde(default)]
    pub listenbrainz: Option<ListenBrainzConfig>,
    #[serde(default)]
    pub lastfm: Option<LastFmConfig>,
```

- [ ] **Step 2: Update existing config tests for the `Option` wrapper**

In `src/config.rs`'s test module, update the four existing tests' assertions:

In `parses_valid_config`, change:
```rust
        assert_eq!(cfg.koito.base_url, "http://koito.example.com");
        assert_eq!(cfg.listenbrainz.user_token, "lb-token");
        assert_eq!(cfg.lastfm.api_key, "lfm-key");
```
to:
```rust
        assert_eq!(cfg.koito.unwrap().base_url, "http://koito.example.com");
        assert_eq!(cfg.listenbrainz.unwrap().user_token, "lb-token");
        assert_eq!(cfg.lastfm.unwrap().api_key, "lfm-key");
```

In `parses_forward_now_playing_flags`, change:
```rust
        assert_eq!(cfg.koito.forward_now_playing, Some(true));
        assert_eq!(cfg.listenbrainz.forward_now_playing, Some(false));
        assert_eq!(cfg.lastfm.forward_now_playing, None); // omitted → None
```
to:
```rust
        assert_eq!(cfg.koito.unwrap().forward_now_playing, Some(true));
        assert_eq!(cfg.listenbrainz.unwrap().forward_now_playing, Some(false));
        assert_eq!(cfg.lastfm.unwrap().forward_now_playing, None); // omitted → None
```

`parses_plex_and_jellyfin_auth_config` and `plex_and_jellyfin_default_to_no_token_when_section_absent` only assert on `cfg.plex`/`cfg.jellyfin` — no change needed to their assertions, but both still include `[koito]`, `[listenbrainz]`, `[lastfm]` sections in their TOML fixtures, which continue to parse into `Some(...)` — leave the TOML fixtures as-is.

- [ ] **Step 3: Add new config tests for partial/absent target configuration**

Add to `src/config.rs`'s test module:

```rust
    #[test]
    fn target_sections_are_all_optional() {
        let toml = r#"
[server]
port = 4567
"#;
        let cfg: Config = toml::from_str(toml).expect("should parse");
        assert!(cfg.koito.is_none());
        assert!(cfg.listenbrainz.is_none());
        assert!(cfg.lastfm.is_none());
    }

    #[test]
    fn partial_target_configuration_parses() {
        let toml = r#"
[server]
port = 4567

[koito]
base_url = "http://koito.example.com"
api_key = "koito-key"

[listenbrainz]
user_token = "lb-token"
"#;
        let cfg: Config = toml::from_str(toml).expect("should parse");
        assert!(cfg.koito.is_some());
        assert!(cfg.listenbrainz.is_some());
        assert!(cfg.lastfm.is_none());
    }
```

- [ ] **Step 4: Add `build_targets()` to `src/targets/mod.rs`**

Add (after the `ScrobbleTarget` trait definition added in Task 1, before `pub async fn fan_out`):

```rust
pub fn build_targets(cfg: &Config, client: reqwest::Client) -> Vec<Arc<dyn ScrobbleTarget>> {
    let mut targets: Vec<Arc<dyn ScrobbleTarget>> = Vec::new();
    if let Some(k) = &cfg.koito {
        targets.push(Arc::new(koito::KoitoTarget::from_config(k, client.clone())));
    }
    if let Some(lb) = &cfg.listenbrainz {
        targets.push(Arc::new(listenbrainz::ListenBrainzTarget::from_config(lb, client.clone())));
    }
    if let Some(lfm) = &cfg.lastfm {
        targets.push(Arc::new(lastfm::LastFmTarget::from_config(lfm, client.clone())));
    }
    targets
}
```

- [ ] **Step 5: Replace `fan_out`/`fan_out_now_playing` with the shared `dispatch()` driver**

In `src/targets/mod.rs`, replace the entire existing bodies of `pub async fn fan_out(cfg: Arc<Config>, client: reqwest::Client, event: PlayEvent)` and `pub async fn fan_out_now_playing(cfg: Arc<Config>, client: reqwest::Client, event: NowPlayingEvent)` (the versions with three hand-written `tokio::spawn` blocks each) with:

```rust
async fn dispatch<E, F, Fut>(targets: Vec<Arc<dyn ScrobbleTarget>>, event: E, join: bool, call: F)
where
    E: Send + Sync + 'static,
    F: Fn(Arc<dyn ScrobbleTarget>, Arc<E>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let event = Arc::new(event);
    let handles: Vec<_> = targets
        .into_iter()
        .map(|t| {
            let event = event.clone();
            tokio::spawn(call(t, event))
        })
        .collect();
    if join {
        for h in handles {
            let _ = h.await;
        }
    }
}

pub async fn fan_out(targets: Vec<Arc<dyn ScrobbleTarget>>, event: PlayEvent) {
    dispatch(targets, event, true, |t, event| async move {
        t.submit_with_retry(&event).await;
    })
    .await;
}

pub async fn fan_out_now_playing(targets: Vec<Arc<dyn ScrobbleTarget>>, event: NowPlayingEvent) {
    dispatch(targets, event, false, |t, event| async move {
        if let Err(e) = t.submit_now_playing(&event).await {
            eprintln!("[NOW-FAIL] {} → {} | {}", event.source, t.name(), e);
        }
    })
    .await;
}
```

The `Config` import (`use crate::config::Config`) is still needed for `build_targets`'s signature — keep the existing `use` line at the top of the file as-is.

- [ ] **Step 6: Rewrite the `fan_out`/`fan_out_now_playing` tests in `src/targets/mod.rs`**

Replace the existing `fan_out_now_playing_does_not_panic_when_all_disabled` and `fan_out_now_playing_spawns_when_enabled` tests (which construct a full `Config` and call `fan_out_now_playing(cfg, client, event)`) with:

```rust
#[tokio::test]
async fn fan_out_now_playing_does_not_panic_when_all_disabled() {
    let cfg = Arc::new(Config {
        server: ServerConfig { port: 4567, webhook_token: None },
        plex: PlexConfig { webhook_token: None },
        jellyfin: JellyfinConfig { webhook_token: None },
        koito: Some(KoitoConfig {
            base_url: "http://localhost:1".to_string(),
            api_key: "k".to_string(),
            forward_now_playing: Some(false),
        }),
        listenbrainz: Some(ListenBrainzConfig {
            user_token: "l".to_string(),
            forward_now_playing: Some(false),
        }),
        lastfm: Some(LastFmConfig {
            api_key: "a".to_string(),
            shared_secret: "s".to_string(),
            session_key: "k".to_string(),
            forward_now_playing: Some(false),
        }),
    });
    let targets = build_targets(&cfg, reqwest::Client::new());
    let event = NowPlayingEvent {
        artist: "Test".to_string(),
        album: None,
        track: "Track".to_string(),
        duration_secs: None,
        source: Source::Navidrome,
    };
    fan_out_now_playing(targets, event).await;
}

#[tokio::test]
async fn fan_out_now_playing_spawns_when_enabled() {
    let cfg = Arc::new(Config {
        server: ServerConfig { port: 4567, webhook_token: None },
        plex: PlexConfig { webhook_token: None },
        jellyfin: JellyfinConfig { webhook_token: None },
        koito: Some(KoitoConfig {
            base_url: "http://localhost:1".to_string(),
            api_key: "k".to_string(),
            forward_now_playing: Some(false),
        }),
        listenbrainz: Some(ListenBrainzConfig {
            user_token: "l".to_string(),
            forward_now_playing: Some(true), // enabled
        }),
        lastfm: Some(LastFmConfig {
            api_key: "a".to_string(),
            shared_secret: "s".to_string(),
            session_key: "k".to_string(),
            forward_now_playing: Some(false),
        }),
    });
    let targets = build_targets(&cfg, reqwest::Client::new());
    let event = NowPlayingEvent {
        artist: "Test".to_string(),
        album: None,
        track: "Track".to_string(),
        duration_secs: None,
        source: Source::Navidrome,
    };
    // Should complete without panicking even though the spawned LB request will fail
    // (localhost:1 is unreachable — the spawn is fire-and-forget so this still returns)
    fan_out_now_playing(targets, event).await;
}

#[tokio::test]
async fn fan_out_joins_all_spawned_tasks() {
    let calls_a = Arc::new(AtomicUsize::new(0));
    let calls_b = Arc::new(AtomicUsize::new(0));
    let targets: Vec<Arc<dyn ScrobbleTarget>> = vec![
        Arc::new(CountingTarget {
            submit_calls: calls_a.clone(),
            submit_now_playing_calls: Arc::new(AtomicUsize::new(0)),
            fail_submit: false,
        }),
        Arc::new(CountingTarget {
            submit_calls: calls_b.clone(),
            submit_now_playing_calls: Arc::new(AtomicUsize::new(0)),
            fail_submit: false,
        }),
    ];
    fan_out(targets, test_play_event()).await;
    assert_eq!(calls_a.load(Ordering::SeqCst), 1);
    assert_eq!(calls_b.load(Ordering::SeqCst), 1);
}

#[test]
fn build_targets_skips_unconfigured_lastfm() {
    let cfg = Config {
        server: ServerConfig { port: 4567, webhook_token: None },
        plex: PlexConfig { webhook_token: None },
        jellyfin: JellyfinConfig { webhook_token: None },
        koito: Some(KoitoConfig {
            base_url: "http://k".to_string(),
            api_key: "k".to_string(),
            forward_now_playing: None,
        }),
        listenbrainz: Some(ListenBrainzConfig {
            user_token: "l".to_string(),
            forward_now_playing: None,
        }),
        lastfm: None,
    };
    let targets = build_targets(&cfg, reqwest::Client::new());
    assert_eq!(targets.len(), 2);
    let names: Vec<&str> = targets.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"Koito"));
    assert!(names.contains(&"ListenBrainz"));
    assert!(!names.contains(&"Last.fm"));
}

#[test]
fn build_targets_returns_empty_when_none_configured() {
    let cfg = Config {
        server: ServerConfig { port: 4567, webhook_token: None },
        plex: PlexConfig { webhook_token: None },
        jellyfin: JellyfinConfig { webhook_token: None },
        koito: None,
        listenbrainz: None,
        lastfm: None,
    };
    let targets = build_targets(&cfg, reqwest::Client::new());
    assert!(targets.is_empty());
}
```

Add the missing imports at the top of the `#[cfg(test)] mod tests` block (alongside `use super::*;`):

```rust
use crate::config::{JellyfinConfig, KoitoConfig, LastFmConfig, ListenBrainzConfig, PlexConfig, ServerConfig};
use crate::event::{NowPlayingEvent, Source};
```

(These replace the old `minimal_cfg()` helper function, which is no longer needed — remove it.)

- [ ] **Step 7: Update `AppState` and handler call sites in `src/router.rs`**

Change:

```rust
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub client: reqwest::Client,
}
```

to:

```rust
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub targets: Vec<Arc<dyn targets::ScrobbleTarget>>,
}
```

In `navidrome_handler`, change:
```rust
                tokio::spawn(targets::fan_out_now_playing(state.cfg, state.client, event));
```
to:
```rust
                tokio::spawn(targets::fan_out_now_playing(state.targets.clone(), event));
```
and change:
```rust
            tokio::spawn(targets::fan_out(state.cfg, state.client, event));
```
to:
```rust
            tokio::spawn(targets::fan_out(state.targets.clone(), event));
```

In `plex_handler`, change both occurrences the same way:
```rust
                    tokio::spawn(targets::fan_out_now_playing(state.cfg, state.client, event));
```
→
```rust
                    tokio::spawn(targets::fan_out_now_playing(state.targets.clone(), event));
```
and:
```rust
                    tokio::spawn(targets::fan_out(state.cfg, state.client, event));
```
→
```rust
                    tokio::spawn(targets::fan_out(state.targets.clone(), event));
```

In `jellyfin_handler`, change both occurrences the same way (identical pattern to `plex_handler`).

- [ ] **Step 8: Update router test fixtures in `src/router.rs`**

Replace `test_app_plex_token`, `test_app_jellyfin_token`, and `test_app_plex_nowplaying` to build `targets` via `targets::build_targets` and wrap `koito`/`listenbrainz`/`lastfm` configs in `Some(...)`. For example, `test_app_plex_token` becomes:

```rust
    fn test_app_plex_token(plex_token: Option<&str>) -> Router {
        let cfg = Config {
            server: crate::config::ServerConfig { port: 4567, webhook_token: None },
            plex: crate::config::PlexConfig {
                webhook_token: plex_token.map(|s| s.to_string()),
            },
            jellyfin: crate::config::JellyfinConfig { webhook_token: None },
            koito: Some(crate::config::KoitoConfig {
                base_url: "http://k".into(),
                api_key: "k".into(),
                forward_now_playing: None,
            }),
            listenbrainz: Some(crate::config::ListenBrainzConfig {
                user_token: "t".into(),
                forward_now_playing: None,
            }),
            lastfm: Some(crate::config::LastFmConfig {
                api_key: "a".into(),
                shared_secret: "s".into(),
                session_key: "k".into(),
                forward_now_playing: None,
            }),
        };
        let targets = targets::build_targets(&cfg, reqwest::Client::new());
        build_router(AppState { cfg: Arc::new(cfg), targets })
    }
```

Apply the same pattern (owned `Config` built first, `targets::build_targets(&cfg, reqwest::Client::new())` called on it, then `Arc::new(cfg)` moved into `AppState`) to `test_app_jellyfin_token` and `test_app_plex_nowplaying`, preserving each function's existing `forward_now_playing` values (`None` for the token tests, `Some(false)` for all three in `test_app_plex_nowplaying`).

- [ ] **Step 9: Update `src/main.rs`**

Change:

```rust
    let port = cfg.server.port;
    let cfg = Arc::new(cfg);
    let client = reqwest::Client::new();

    let state = router::AppState { cfg, client };
```

to:

```rust
    let port = cfg.server.port;
    let client = reqwest::Client::new();
    let targets = targets::build_targets(&cfg, client.clone());
    let cfg = Arc::new(cfg);

    let state = router::AppState { cfg, targets };
```

- [ ] **Step 10: Run the full test suite**

Run: `cargo test`
Expected: PASS — every test in `src/config.rs`, `src/targets/mod.rs`, `src/targets/koito.rs`, `src/targets/listenbrainz.rs`, `src/targets/lastfm.rs`, and `src/router.rs` green.

- [ ] **Step 11: Run a build check**

Run: `cargo build`
Expected: builds cleanly with no warnings about unused `client` field or unused imports.

- [ ] **Step 12: Commit**

```bash
git add src/config.rs src/targets/mod.rs src/router.rs src/main.rs
git commit -m "feat: wire ScrobbleTarget objects through config, fan-out, and router"
```

---

### Task 6: Update documentation and final verification

**Files:**
- Modify: `config.toml.example`
- Modify: `README.md`
- Modify: `CLAUDE.md`

**Interfaces:**
- Consumes: nothing new — this task only updates prose to reflect the behavior implemented in Tasks 1-5.

- [ ] **Step 1: Update `config.toml.example`**

Add a one-line comment above each target section noting it's optional. Change:

```toml
[koito]
base_url = "http://koito.yourdomain.com"
api_key  = "your-koito-api-key"
```

to:

```toml
# [koito], [listenbrainz], and [lastfm] are all optional — omit any section
# entirely to disable scrobbling to that target.
[koito]
base_url = "http://koito.yourdomain.com"
api_key  = "your-koito-api-key"
```

- [ ] **Step 2: Update `README.md`**

In the section describing targets (around the `| Target | Protocol |` table), add a sentence directly after the table:

```markdown
Each of `[koito]`, `[listenbrainz]`, and `[lastfm]` in `config.toml` is optional — omit any section to disable scrobbling to that target. At least one should typically be configured, but this isn't enforced.
```

- [ ] **Step 3: Update `CLAUDE.md`**

In the "Architecture" section, update the "Targets" bullet to describe the trait-based design. Change:

```markdown
**Targets** (`src/targets/`): Each exposes `submit_to(base_url, credentials, client, event)` for testability with mockito. Koito and ListenBrainz share `build_lb_payload()` from `listenbrainz.rs`. Last.fm uses MD5 signature via `BTreeMap` (alphabetical param ordering guaranteed).
```

to:

```markdown
**Targets** (`src/targets/`): Each target implements the `ScrobbleTarget` trait (`name`, `submit`, `submit_now_playing` with a no-op default, `submit_with_retry` with a shared default retry policy) — see `src/targets/mod.rs`. `KoitoTarget`, `ListenBrainzTarget`, `LastFmTarget` each own their config + a cloned `reqwest::Client`, constructed via `from_config`. `build_targets(&Config, Client) -> Vec<Arc<dyn ScrobbleTarget>>` constructs only the targets whose `config.toml` section is present — each of `[koito]`, `[listenbrainz]`, `[lastfm]` is independently optional. Each still exposes `submit_to(base_url, credentials, client, event)` free functions for testability with mockito. Koito and ListenBrainz share `build_lb_payload()` from `listenbrainz.rs`. Last.fm uses MD5 signature via `BTreeMap` (alphabetical param ordering guaranteed).
```

Update the "Fan-out" bullet. Change:

```markdown
**Fan-out** (`src/targets/mod.rs`): `fan_out` spawns 3 tasks concurrently with retry (1s → 4s backoff, joined). `fan_out_now_playing` is fire-and-forget (tasks spawned, not joined, no retry) — `[NOW-FAIL]` on error. Koito now-playing defaults to off (`forward_now_playing = false`); ListenBrainz and Last.fm default to on.
```

to:

```markdown
**Fan-out** (`src/targets/mod.rs`): `fan_out`/`fan_out_now_playing` are thin wrappers around a shared `dispatch()` helper that spawns one task per target in `AppState.targets`. `fan_out` joins all tasks (retry via each target's `submit_with_retry` default, 1s → 4s backoff). `fan_out_now_playing` does not join (fire-and-forget, no retry) — `[NOW-FAIL]` on error. Adding a new target requires only a new struct + `ScrobbleTarget` impl + one line in `build_targets()` — no changes to `dispatch`, `fan_out`, or `fan_out_now_playing`. Koito now-playing defaults to off (`forward_now_playing = false`); ListenBrainz and Last.fm default to on.
```

- [ ] **Step 4: Run the full test suite one final time**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add config.toml.example README.md CLAUDE.md
git commit -m "docs: document optional target sections and ScrobbleTarget trait"
```
