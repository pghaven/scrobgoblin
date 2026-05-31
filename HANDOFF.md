# Scroblin Handoff

## Current Status (2026-05-31)

Scroblin is deployed and working at `http://scroblin:4567` (internal) / `https://scroblin.geary.quest` (external via Traefik). Navidrome is configured to use it as its ListenBrainz endpoint (`ND_LISTENBRAINZ_BASEURL=http://host.docker.internal:4567`). All three targets (Koito, ListenBrainz, Last.fm) are receiving scrobbles successfully. Memory footprint is ~8 MB, CPU is ~0% at idle.

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

**Root cause:** The ListenBrainz API spec requires a `{"status": "ok"}` JSON response body on successful submission. Scroblin was returning `StatusCode::OK` (HTTP 200) with an empty body. Navidrome's ListenBrainz client reads the response body and, finding no content (EOF), treats the request as failed and re-queues the scrobble for retry. This also affected `playing_now` events (`updateNowPlaying returned error=EOF`).

**Fix:** Changed `navidrome_handler` return type from `StatusCode` to `impl IntoResponse`. Added a `lb_ok()` helper that returns `(StatusCode::OK, Json({"status": "ok"}))`. All success paths now return this. Error paths return `(StatusCode::BAD_REQUEST, Json({"status": "error"}))`.

**Aftermath:** Fixing this caused Navidrome to immediately flush its retry queue — approximately 2 hours of backed-up scrobbles were submitted in a burst. All three targets received the full backlog correctly.

---

## Deployment Notes

- Container: `scroblin`, port 4567, `restart: unless-stopped`
- Config: `./config.toml` mounted read-only at `/app/config.toml`
- Network: `proxiable` (Traefik), plus `extra_hosts: koito.geary.quest:host-gateway` for internal routing
- No mem_limit set — footprint is ~8 MB, well within safe range
- Navidrome env vars: `ND_LISTENBRAINZ_BASEURL=http://host.docker.internal:4567`, `ND_LISTENBRAINZ_APIKEY=<any non-empty value>`
