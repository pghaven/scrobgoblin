# Scroblin Handoff

## Current Status (2026-05-31)

Scroblin is deployed and working at `http://scroblin:4567` (internal) / `https://scroblin.geary.quest` (external via Traefik). Navidrome is configured to use it as its ListenBrainz endpoint. All three targets (Koito, ListenBrainz, Last.fm) are receiving scrobbles and now-playing notifications successfully. Memory footprint is ~8 MB, CPU is ~0% at idle.

---

## Session 1 — Initial Build (2026-05-31)

Built and deployed the full project per the plan at `docs/superpowers/plans/2026-05-31-scroblin.md`.

---

## Session 2 — Bug Fixes (2026-05-31)

Three bugs were discovered and fixed during initial production testing.

### Bug 1: Wrong Koito API path

**Symptom:** Scrobbles to Koito were silently failing.

**Root cause:** The Koito submit endpoint is `/apis/listenbrainz/1/submit-listens`, not `/1/submit-listens`. Koito's ListenBrainz-compatible API lives under `/apis/listenbrainz/1/`, not at the root.

**Fix:** `src/targets/koito.rs` — changed the POST URL from `{base_url}/1/submit-listens` to `{base_url}/apis/listenbrainz/1/submit-listens`. Tests updated to match.

---

### Bug 2: Navidrome `playing_now` events submitted as scrobbles

**Symptom:** Koito showed the same song scrobbled repeatedly while other targets showed the correct current song. Appeared as though one song was "stuck" looping.

**Root cause:** Navidrome sends two event types to `/1/submit-listens`:
- `listen_type: "playing_now"` when a track starts playing
- `listen_type: "single"` when the track qualifies as a scrobble

The original `LbPayload` struct did not capture `listen_type`, so both event types were treated as scrobbles and fanned out to all targets. Last.fm and ListenBrainz deduplicate these silently; Koito does not, resulting in multiple identical entries per song play.

**Fix:** Added `listen_type: Option<String>` to `LbPayload` in `src/sources/navidrome.rs`. In `navidrome_handler` (`src/router.rs`), added an early return for `playing_now` events before the fan-out.

---

### Bug 3: Empty response body caused Navidrome infinite retry loop

**Symptom:** The same song ("Dido - Sand in My Shoes") was scrobbled to all three targets continuously and indefinitely, even when nothing was playing. Navidrome logs showed `ListenBrainz Scrobble returned HTTP error error=EOF` and `Could not send scrobble. Will be retried` in a tight loop. The `listened_at` timestamp was identical on every submission — the same queued event being replayed.

**Root cause:** The ListenBrainz API spec requires a `{"status": "ok"}` JSON response body on successful submission. Scroblin was returning `StatusCode::OK` (HTTP 200) with an empty body. Navidrome's ListenBrainz client reads the response body and, finding no content (EOF), treats the request as failed and re-queues the scrobble for retry.

**Fix:** Changed `navidrome_handler` return type from `StatusCode` to `impl IntoResponse`. Added a `lb_ok()` helper that returns `(StatusCode::OK, Json({"status": "ok"}))`. All success paths now return this.

**Aftermath:** Fixing this caused Navidrome to immediately flush its retry queue — approximately 2 hours of backed-up scrobbles were submitted in a burst. All three targets received the full backlog correctly.

---

## Session 3 — Now-Playing Forwarding (2026-05-31)

Implemented per-target "now playing" forwarding per the plan at `docs/superpowers/plans/2026-05-31-now-playing.md`.

### What was built

Previously `playing_now` events from Navidrome were silently dropped. Now they are forwarded to all configured targets:

- **ListenBrainz:** forwards as `listen_type: "playing_now"` to `/1/submit-listens` (default on)
- **Last.fm:** calls `track.updateNowPlaying` via `ws.audioscrobbler.com/2.0/` (default on)
- **Koito:** disabled by default pending confirmation of deduplication behaviour

### Key implementation details

- New `NowPlayingEvent` struct in `src/event.rs` — mirrors `PlayEvent` but without `played_at`
- Per-target `forward_now_playing: Option<bool>` config flags in each target config struct
- `parse_now_playing()` in `src/sources/navidrome.rs` extracts track info from LB payload
- `build_now_playing_payload()` in `src/targets/listenbrainz.rs` — reused by Koito (DRY, same LB wire format)
- Last.fm `track.updateNowPlaying` uses bare param names (`artist`, `track`) without `[0]` suffix — different from `track.scrobble`
- `fan_out_now_playing()` in `src/targets/mod.rs` — fire-and-forget (tasks spawned, not joined); failures logged with `[NOW-FAIL]` prefix, not retried
- Router wiring: `playing_now` events parsed and dispatched before the scrobble path; always returns `lb_ok()` regardless of parse outcome

### Config additions

```toml
[koito]
# forward_now_playing = false  # default false

[listenbrainz]
# forward_now_playing = true   # default true

[lastfm]
# forward_now_playing = true   # default true
```

---

## Deployment Notes

- Container: `scroblin`, port 4567, `restart: unless-stopped`
- Config: `./config.toml` mounted read-only at `/app/config.toml`
- Network: `proxiable` (Traefik), plus `extra_hosts: koito.geary.quest:host-gateway` for internal routing
- No mem_limit set — footprint is ~8 MB, well within safe range
- Navidrome env vars: `ND_LISTENBRAINZ_BASEURL=http://host.docker.internal:4567`, `ND_LISTENBRAINZ_APIKEY=<webhook_token value>`

## Next Steps

See ROADMAP.md. Priority order:
1. Plex and Jellyfin webhook authentication (low effort, real security gap — Scroblin is externally exposed via Traefik)
2. Koito now-playing: test Koito deduplication, then enable `forward_now_playing = true` in config if confirmed
3. Structured logging (`tracing` crate) — nice-to-have for log aggregation
