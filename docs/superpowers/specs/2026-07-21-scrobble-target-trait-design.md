# Scrobble Target Trait Design

**Date:** 2026-07-21

## Goal

Scrobgoblin currently hardcodes exactly three scrobble targets (Koito, ListenBrainz, Last.fm), all of which are mandatory in `config.toml` and all of which are unconditionally invoked by `fan_out`/`fan_out_now_playing`. This design makes each target independently configurable (any subset, including none) and restructures targets as self-contained objects behind a shared trait, so that adding a new target in the future means writing a new target implementation — not modifying shared fan-out logic.

This design covers **targets only** (issue #3, retitled/rescoped for this purpose). Source-side flexibility (Navidrome/Plex/Jellyfin) is out of scope here and tracked separately in issue #10, to be designed later using lessons from this effort.

## Context

Today:
- `src/config.rs`: `koito: KoitoConfig`, `listenbrainz: ListenBrainzConfig`, `lastfm: LastFmConfig` are all required fields (no `Option`, no `#[serde(default)]`) — omitting any section fails config parsing at startup.
- `src/targets/mod.rs`: `fan_out()` unconditionally spawns three tasks calling `koito::submit`, `listenbrainz::submit`, `lastfm::submit`, with a hand-written retry loop (3 attempts, 1s→4s backoff) duplicated per task. `fan_out_now_playing()` unconditionally spawns three tasks (fire-and-forget, no retry), each individually gated by `cfg.<target>.forward_now_playing.unwrap_or(...)`.
- Each target module (`src/targets/koito.rs`, `listenbrainz.rs`, `lastfm.rs`) exposes a free function `submit_to(base_url, credentials, client, event)` for mockito-based testing.

## Design

### 1. The `ScrobbleTarget` trait

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

    /// Default method: shared retry policy (3 attempts, 1s -> 4s backoff),
    /// calling self.submit() and using self.name() for [OK]/[FAIL] logging.
    /// Individual targets may override if a specific API needs different
    /// retry behavior; none do today.
    async fn submit_with_retry(&self, event: &PlayEvent) {
        for attempt in 1..=3u32 {
            match self.submit(event).await {
                Ok(()) => {
                    println!("[OK] {} → {} | {} - {}", event.source, self.name(), event.artist, event.track);
                    return;
                }
                Err(e) => {
                    let delays = [1u64, 4];
                    if attempt < 3 {
                        eprintln!("[FAIL] {} → {} | {} - {} | attempt {}/3 | {} | retrying in {}s",
                            event.source, self.name(), event.artist, event.track, attempt, e, delays[(attempt - 1) as usize]);
                        tokio::time::sleep(tokio::time::Duration::from_secs(delays[(attempt - 1) as usize])).await;
                    } else {
                        eprintln!("[FAIL] {} → {} | {} - {} | attempt 3/3 | {}",
                            event.source, self.name(), event.artist, event.track, e);
                    }
                }
            }
        }
    }
}
```

Adds a new dependency: `async-trait` (compile-time-only macro, standard solution for dyn-compatible async trait methods; no runtime behavior change).

Each concrete target owns its own config fields plus a cloned `reqwest::Client`:

```rust
pub struct KoitoTarget { base_url: String, api_key: String, forward_now_playing: bool, client: reqwest::Client }
pub struct ListenBrainzTarget { user_token: String, forward_now_playing: bool, client: reqwest::Client }
pub struct LastFmTarget { api_key: String, shared_secret: String, session_key: String, forward_now_playing: bool, client: reqwest::Client }
```

`forward_now_playing` (currently `Option<bool>` on each config struct, defaulted differently per target — Koito off, ListenBrainz/Last.fm on) is resolved to a concrete `bool` at construction time and checked inside each target's own `submit_now_playing` impl. Koito's default-off behavior becomes internal logic in `KoitoTarget::submit_now_playing`, not a caller-side check.

The internals of `submit`/`submit_now_playing` (HTTP request construction, payload building, signature logic for Last.fm, etc.) move near-verbatim from today's `submit_to`-style free functions in `src/targets/koito.rs`/`listenbrainz.rs`/`lastfm.rs` — this is a restructuring of call shape, not a rewrite of target-specific logic.

### 2. Construction & registry

```rust
pub fn build_targets(cfg: &Config, client: reqwest::Client) -> Vec<Arc<dyn ScrobbleTarget>> {
    let mut targets: Vec<Arc<dyn ScrobbleTarget>> = Vec::new();
    if let Some(k) = &cfg.koito {
        targets.push(Arc::new(KoitoTarget::from_config(k, client.clone())));
    }
    if let Some(lb) = &cfg.listenbrainz {
        targets.push(Arc::new(ListenBrainzTarget::from_config(lb, client.clone())));
    }
    if let Some(lfm) = &cfg.lastfm {
        targets.push(Arc::new(LastFmTarget::from_config(lfm, client.clone())));
    }
    targets
}
```

- `Config`'s `koito`, `listenbrainz`, `lastfm` fields become `Option<KoitoConfig>`, `Option<ListenBrainzConfig>`, `Option<LastFmConfig>` with `#[serde(default)]` — matching the existing `PlexConfig`/`JellyfinConfig` optional-section pattern. Omitting a `[lastfm]` section in `config.toml` means no `LastFmTarget` is ever constructed; no per-call checks needed anywhere downstream.
- `build_targets()` runs once in `main.rs` at startup (not per-request). `AppState` gains a `targets: Vec<Arc<dyn ScrobbleTarget>>` field, alongside the existing `Arc<Config>` (still needed for `server`/`plex`/`jellyfin` source-side config). Cloning `AppState` clones the `Vec` (a handful of pointer-sized `Arc` clones — negligible, and no different in cost from `AppState`'s existing `reqwest::Client` clone, which is itself `Arc`-backed internally).
- **Adding a new target in the future** requires: (1) a new struct + trait impl + `from_config` constructor, (2) one new `Option<...Config>` field on `Config`, (3) one new `if let Some(...)` line in `build_targets()`. No changes to the trait, `dispatch()`, `fan_out`, `fan_out_now_playing`, or any existing target.

### 3. Fan-out driver

```rust
async fn dispatch<E, F, Fut>(targets: Vec<Arc<dyn ScrobbleTarget>>, event: E, join: bool, call: F)
where
    E: Send + Sync + 'static,
    F: Fn(Arc<dyn ScrobbleTarget>, Arc<E>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let event = Arc::new(event);
    let handles: Vec<_> = targets.into_iter()
        .map(|t| { let event = event.clone(); tokio::spawn(call(t, event)) })
        .collect();
    if join {
        let _ = futures::future::join_all(handles).await;
    }
}

pub async fn fan_out(targets: Vec<Arc<dyn ScrobbleTarget>>, event: PlayEvent) {
    dispatch(targets, event, true, |t, event| async move {
        t.submit_with_retry(&event).await;
    }).await;
}

pub async fn fan_out_now_playing(targets: Vec<Arc<dyn ScrobbleTarget>>, event: NowPlayingEvent) {
    dispatch(targets, event, false, |t, event| async move {
        if let Err(e) = t.submit_now_playing(&event).await {
            eprintln!("[NOW-FAIL] {} → {} | {}", event.source, t.name(), e);
        }
    }).await;
}
```

- `fan_out` joins all spawned tasks (same behavior as today — needed so tests can `.await` completion of all targets' retries). `fan_out_now_playing` does not join (fire-and-forget, same as today).
- Both functions become entirely target-agnostic — no target names appear in `src/targets/mod.rs` except inside log lines constructed from `t.name()`.
- The join/no-join distinction and the choice of trait method (`submit_with_retry` vs `submit_now_playing`) are the *only* differences between the two public functions; everything else is shared via `dispatch()`.
- Now-playing gets **no retry** (single attempt, matching current behavior) — a stale now-playing update has little value once the track has likely changed, so retrying isn't worth the complexity.
- Router call sites in `src/router.rs` change from e.g. `targets::fan_out(state.cfg, state.client, event)` to `targets::fan_out(state.targets.clone(), event)`.

### 4. Error handling & logging

- `submit_with_retry` (trait default method) owns `[OK]`/`[FAIL]` logging, using `self.name()` instead of a hardcoded string literal per call site. Retry policy (3 attempts, 1s→4s backoff) is centralized once instead of duplicated three times.
- `[NOW-FAIL]` logging stays in the `dispatch()` closure passed by `fan_out_now_playing`, since it's specific to that dispatch path rather than something every target needs to reimplement.
- Each target's `submit`/`submit_now_playing` returns `anyhow::Result<()>`, matching today's free functions.

### 5. Testing strategy

- Each target's existing mockito-based unit tests move essentially unchanged — they construct a target instance and call `submit`/`submit_now_playing` via the trait method instead of a free function.
- `build_targets()` gets new tests verifying optional-config behavior (e.g. `Config` with `lastfm: None` produces a `Vec` without a `LastFmTarget`, checked via `.name()` or vec length).
- `dispatch()`/`fan_out`/`fan_out_now_playing` get tests using small fake `ScrobbleTarget` implementations (e.g. a `CountingTarget` that records calls) rather than requiring real Koito/LB/Last.fm configs — this is easier to test than today's setup, since dispatch logic no longer needs real target wiring to exercise.
- Router test fixtures (`test_app_plex_token`, `test_app_jellyfin_token`, `test_app_plex_nowplaying` in `router.rs`) update their `Config`/`AppState` construction for the new `Option<...>` config fields and the `targets` field on `AppState`.

## Out of Scope

- Source-side flexibility (pluggable Navidrome/Plex/Jellyfin sources) — tracked in issue #10.
- Changing the retry policy's parameters (attempt count, backoff timing) — unchanged from today, just centralized.
- Runtime target reconfiguration (targets are still built once at startup from `config.toml`; no hot-reload).
