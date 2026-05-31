# Scroblin Roadmap

---

## ✅ Done — Webhook token authentication

**Status:** Implemented 2026-05-31.

The `validate-token` and `submit-listens` handlers now check the `Authorization: Token <value>` header against `server.webhook_token` in `config.toml`. This matches the token Navidrome already sends via `ND_LISTENBRAINZ_APIKEY`. If the token is unset, all requests are allowed (safe for internal-only deployments). If set, mismatched requests receive HTTP 401.

This closes the public-exposure risk from the Traefik routing at `https://scroblin.geary.quest`.

---

## 1 — Per-target "now playing" forwarding

**Status:** Not started.

### Background

Navidrome sends two event types to `/1/submit-listens`:
- `listen_type: "playing_now"` — track started; no scrobble yet
- `listen_type: "single"` — track completed; actual scrobble

Currently Scroblin drops `playing_now` events entirely. This is correct for Koito and Last.fm (which don't support it well or deduplicate poorly), but wasteful for ListenBrainz, which has a proper "now playing" display powered by these events.

Last.fm also has a `track.updateNowPlaying` API method that updates the "now playing" widget on a user's profile. Koito's LB-compatible API may accept `playing_now` submissions — untested.

### Proposed behaviour

Add per-target flags to `config.toml` to control whether `playing_now` events are forwarded:

```toml
[listenbrainz]
user_token = "..."
forward_now_playing = true   # default true — LB handles it natively

[lastfm]
...
forward_now_playing = true   # default true — uses track.updateNowPlaying

[koito]
...
forward_now_playing = false  # default false — Koito doesn't deduplicate
```

### Implementation notes

- **ListenBrainz:** Forward `playing_now` as-is to `/1/submit-listens` with `listen_type: "playing_now"`. No `listened_at` needed.
- **Last.fm:** Call `track.updateNowPlaying` with the same MD5-signed form params as `track.scrobble`, but without `timestamp[0]`. Duration is optional.
- **Koito:** Off by default until confirmed that Koito deduplicates or shows now-playing correctly. Test by enabling and watching the Koito UI.
- `fan_out` needs to branch on `event.is_now_playing: bool` (or a separate `NowPlayingEvent` type) to call the right submit function per target.

---

## 2 — Plex and Jellyfin validation

**Status:** Not started.

The Plex and Jellyfin webhook handlers are currently unauthenticated. Both platforms support webhook secrets:
- **Plex:** sends `X-Plex-Token` header — validate against a token in config
- **Jellyfin:** webhook plugin supports a shared secret sent as a custom header — validate against a secret in config

Low priority if the Docker port is not externally reachable, but worth adding for completeness.

---

## 3 — Structured logging

**Status:** Not started.

Replace `println!`/`eprintln!` with `tracing` crate structured logs. Benefits:
- JSON output mode for log aggregation (Loki/Promtail)
- Log levels (`INFO`, `WARN`, `ERROR`) filterable at runtime via `RUST_LOG`
- Consistent field names (`target`, `artist`, `track`, `attempt`, `listened_at`)
