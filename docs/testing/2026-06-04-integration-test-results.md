# Integration Test Results — 2026-06-04

End-to-end integration test covering all three webhook sources (Plex, Jellyfin, Navidrome) and all three scrobble targets (Koito, Last.fm, ListenBrainz).

## Environment

- Scroblin commit: e90456d
- Sources active: Navidrome, Plex, Jellyfin
- Targets active: Koito, Last.fm, ListenBrainz
- Deployment: Docker Compose on minimac (Colima/sshfs)

---

## Plex

### Scrobble
- **Test:** Play a track in Plex to completion (≥30s)
- **Event:** `media.scrobble`
- **Result:** PASS — all three targets confirmed `[OK]`

### Now-playing on play
- **Test:** Start playback in Plex
- **Event:** `media.play`
- **Result:** PASS — ListenBrainz and Last.fm received now-playing update

### Now-playing on resume
- **Test:** Pause then resume a track in Plex
- **Event:** `media.resume`
- **Result:** PASS — ListenBrainz and Last.fm received now-playing update

### titleSort fallback
- **Test:** Play "Take On Me" by a-ha (Plex stores title blank, real name in `titleSort`)
- **Result:** PASS — track name resolved correctly via `effective_title()`

---

## Jellyfin

### Webhook template required
- **Symptom (pre-fix):** Jellyfin Generic Client sent empty body; Scroblin returned 400 (`EOF while parsing JSON`)
- **Fix:** Configured Handlebars template in `Jellyfin.Plugin.Webhook.xml`
- **Result:** PASS — body received and parsed correctly after fix

### Scrobble (web player)
- **Test:** Play a track to completion in the Jellyfin web player
- **Event:** `PlaybackStop` (position > 0)
- **Track tested:** Alice in Chains — Rotten Apple
- **Result:** PASS — `[OK] Jellyfin → Koito`, `[OK] Jellyfin → Last.fm`, `[OK] Jellyfin → ListenBrainz`

### Now-playing (web player)
- **Test:** Start playback in the Jellyfin web player
- **Event:** `PlaybackStart`
- **Track tested:** Alice in Chains — Nutshell
- **Result:** PASS — ListenBrainz and Last.fm received now-playing update; Koito skipped (`forward_now_playing = false`)

### Duplicate scrobble prevention
- **Symptom (pre-fix):** Two `PlaybackStop` events fire per track — one at real position, one at position 0 (session cleanup when next track starts)
- **Fix:** Filter `played_position_ticks == Some(0)` in `parse()`; router silently drops position-0 errors
- **Result:** PASS — single scrobble per track confirmed

### Position-0 event not logged as warning
- **Test:** Trigger playback stop that produces position-0 cleanup event
- **Result:** PASS — no `[WARN]` line emitted; silently ignored

### Mobile clients (Manet iOS, Jellify)
- **Result:** NOTE — these clients do not reliably send `PlaybackStart`/`PlaybackStop` webhook events. Only the Jellyfin web player produces complete event sequences. Mobile scrobbling via Jellyfin is limited by client behavior, not Scroblin.

---

## Navidrome

### Scrobble
- **Pre-existing:** Navidrome scrobbling was confirmed working in an earlier session
- **Result:** PASS (unchanged)

### playing_now filter
- **Pre-existing:** `playing_now` events filtered before fan-out to prevent duplicate submissions
- **Result:** PASS (unchanged)

### Response body `{"status":"ok"}`
- **Pre-existing:** Navidrome requires JSON response body; bare 200 causes EOF error and retry loop
- **Result:** PASS (unchanged)

---

## Known Limitations

- **Jellyfin `{{#if}}` block helpers unsupported:** `HandlebarsDotNet` v2.1.6.0 (bundled with webhook plugin) throws `HandlebarsCompilerException` for any `{{#if}}` block. Webhook template uses plain `{{expression}}` only; optional numeric fields (`RunTimeTicks`, `PlaybackPositionTicks`) rely on Jellyfin always providing them for music events.
- **Mobile Jellyfin clients:** Manet and Jellify do not fire complete webhook event sequences. Web player only for Jellyfin scrobbling.
