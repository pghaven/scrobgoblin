# Scroblin Roadmap

---

## ✅ Done — Webhook token authentication

**Status:** Implemented 2026-05-31.

The `validate-token` and `submit-listens` handlers now check the `Authorization: Token <value>` header against `server.webhook_token` in `config.toml`. This matches the token Navidrome already sends via `ND_LISTENBRAINZ_APIKEY`. If the token is unset, all requests are allowed (safe for internal-only deployments). If set, mismatched requests receive HTTP 401.

This closes the public-exposure risk from the Traefik routing at `https://scroblin.geary.quest`.

---

## ✅ Done — Mobile client scrobbling

**Status:** Confirmed working 2026-05-31.

Scrobbling from both the Navidrome web UI and mobile Subsonic clients is confirmed working.

---

## 1 — Plex and Jellyfin webhook authentication

**Status:** Not started.

The Plex and Jellyfin webhook handlers are currently unauthenticated. Both platforms support webhook secrets:
- **Plex:** sends `X-Plex-Token` header — validate against a token in config
- **Jellyfin:** webhook plugin supports a shared secret sent as a custom header — validate against a secret in config

Low effort, closes a real gap since Scroblin is externally exposed via Traefik.

---

## 2 — Per-target "now playing" forwarding

**Status:** Not started. Implementation plan at `docs/superpowers/plans/2026-05-31-now-playing.md`.

Navidrome sends `listen_type: "playing_now"` to `/1/submit-listens` when a track starts. Scroblin currently drops these silently. Forwarding them enables:
- **ListenBrainz:** "now playing" display on user profile
- **Last.fm:** "now playing" widget via `track.updateNowPlaying`
- **Koito:** optional, off by default until deduplication behaviour is confirmed

Per-target `forward_now_playing` config flags control which targets receive these events. Defaults: ListenBrainz `true`, Last.fm `true`, Koito `false`.

---

## 3 — Structured logging

**Status:** Not started.

Replace `println!`/`eprintln!` with `tracing` crate structured logs. Benefits:
- JSON output mode for log aggregation (Loki/Promtail)
- Log levels (`INFO`, `WARN`, `ERROR`) filterable at runtime via `RUST_LOG`
- Consistent field names (`target`, `artist`, `track`, `attempt`, `listened_at`)
