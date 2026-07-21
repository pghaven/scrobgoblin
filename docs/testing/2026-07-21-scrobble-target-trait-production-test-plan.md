# Production Test Plan: Scrobble Target Trait Refactor

**Audience:** Claude Code running on the Scrobgoblin production host.
**Related work:** `docs/superpowers/specs/2026-07-21-scrobble-target-trait-design.md`, `docs/superpowers/plans/2026-07-21-scrobble-target-trait.md`
**Commit range:** `24197fa..0beeab2` (merged to `main`, pushed to Forgejo)

## What changed, and why this test plan is shaped the way it is

Before this refactor, `[koito]`, `[listenbrainz]`, and `[lastfm]` were all mandatory in `config.toml`, and `fan_out`/`fan_out_now_playing` hardcoded calls to all three. After this refactor:

- Each of `[koito]`, `[listenbrainz]`, `[lastfm]` is **independently optional**. Omitting a section means Scrobgoblin never attempts to scrobble to that target — no error, no log spam, just silent exclusion.
- Each target is now a struct implementing a shared `ScrobbleTarget` trait, constructed once at startup via `build_targets()`.
- The server now logs its active target set at startup (`Active scrobble targets: ...` or a `[WARN]` if none are configured) — this is new, added specifically so a config mistake isn't silent.
- `fan_out` (scrobbles) still joins all target tasks with retry (1s → 4s backoff, 3 attempts); `fan_out_now_playing` is still fire-and-forget with no retry. These behaviors are unchanged from before the refactor — this plan verifies they're *actually* unchanged, not just theoretically unchanged.

This means the test plan has two halves:
1. **New capability**: does the optional-target configuration actually work as intended, end to end, on the real box?
2. **Regression**: does everything that worked before (all three sources, both scrobble and now-playing paths, all three targets, threshold filtering, webhook auth) still work exactly as before?

Do not skip the regression half — the refactor touched `src/router.rs`, `src/main.rs`, and all three target files, so it has broad blast radius even though the design intent was narrow.

## Prerequisites

```bash
cd /path/to/scrobgoblin/checkout   # wherever the production clone lives
git fetch origin
git log --oneline origin/main -5   # confirm 0beeab2 is present
```

Back up the current running config before touching anything:

```bash
cp conf/config.toml conf/config.toml.pre-refactor-backup
```

Note which target sections (`[koito]`, `[listenbrainz]`, `[lastfm]`) are actually present in the live `conf/config.toml` before you start — you'll need this to restore the exact original state at the end, and to know which targets *should* appear in the startup log.

## Phase 0: Deploy

```bash
git pull origin main
docker compose build
docker compose up -d
docker compose logs -f scrobgoblin
```

**Check:** within the startup logs, confirm one of:
- `Active scrobble targets: <comma-separated list>` — and that the list matches exactly the target sections present in `conf/config.toml` (e.g. if only `[koito]` and `[listenbrainz]` are configured, `Last.fm` must NOT appear in this list).
- OR, if somehow no target sections are present, `[WARN] No scrobble targets configured — check config.toml section names ([koito]/[listenbrainz]/[lastfm])`.

If the log line doesn't appear at all, or lists a target that isn't actually configured (or omits one that is), **stop here** — that's a Critical finding, the core feature of this refactor is broken.

## Phase 1: Optional-target configuration (the new behavior)

Do these one at a time, restoring the backup config and restarting between each, so you're never testing more than one config change at once.

### 1a. Baseline — full existing config

Already covered by Phase 0. Confirm scrobbling still works end-to-end for at least one real playback event on whichever music source you have available (Navidrome/Plex/Jellyfin), and that it reaches every currently-configured target. Check each target's own side (Koito web UI listen history, ListenBrainz profile, Last.fm profile/recent scrobbles) rather than trusting Scrobgoblin's logs alone — the goal is confirming the whole pipeline, not just that Scrobgoblin thinks it succeeded.

### 1b. Remove one target section

Pick whichever target is least disruptive to temporarily lose (e.g. Last.fm), and comment out or delete its entire `[section]` block in `conf/config.toml`:

```bash
docker compose restart scrobgoblin
docker compose logs scrobgoblin | tail -20
```

**Check:**
- Startup log's active-target list no longer includes the removed target, and still includes the others.
- Trigger a real scrobble (or use a curl-based test from Phase 2 below). Confirm the remaining targets still receive it (check their own UIs), and confirm **no log line of any kind mentions the removed target** — no `[OK]`, no `[FAIL]`, nothing. Its absence should be completely silent, not an error.
- Confirm the container stays healthy: `docker compose ps` shows it running, not restarting.

### 1c. Remove all target sections

Comment out or delete all three `[koito]`/`[listenbrainz]`/`[lastfm]` blocks (temporarily — this is the "everything unconfigured" edge case):

```bash
docker compose restart scrobgoblin
docker compose logs scrobgoblin | tail -20
```

**Check:**
- Startup log shows `[WARN] No scrobble targets configured ...`.
- Container starts and stays healthy — it must NOT crash, panic, or restart-loop just because there are zero targets.
- Send a test webhook (any of the curl commands in Phase 2). Confirm the server still responds `200 OK` (webhooks are accepted and silently dropped, not rejected) and logs nothing claiming a scrobble succeeded.

### 1d. Restore

```bash
cp conf/config.toml.pre-refactor-backup conf/config.toml
docker compose restart scrobgoblin
docker compose logs scrobgoblin | tail -20
```

**Check:** startup log's active-target list matches the original full set again, exactly as in Phase 0.

## Phase 2: Regression — sources and event paths

These use `curl` directly against the running server so you don't have to wait for a real playback event. Adjust the `Authorization`/token header and the target port/host to match your actual `conf/config.toml` and deployment (internal port 4567, or through Traefik at `https://scrobgoblin.geary.quest` if testing from outside the host).

For each of the three sources below, test **both** the scrobble path and the now-playing path, and **both** a track above the 30-second threshold (should scrobble) and below it (should silently not scrobble) for the scrobble path specifically.

### 2a. Navidrome

Scrobble (`listen_type: "single"`, duration 200s — above threshold):

```bash
curl -i -X POST http://localhost:4567/1/submit-listens \
  -H "Content-Type: application/json" \
  -H "Authorization: Token YOUR_SERVER_WEBHOOK_TOKEN" \
  -d '{
    "listen_type": "single",
    "payload": [{
      "listened_at": '"$(date +%s)"',
      "track_metadata": {
        "artist_name": "Test Artist",
        "track_name": "Test Track (scrobble-target-trait regression check)",
        "release_name": "Test Album",
        "additional_info": { "duration": 200 }
      }
    }]
  }'
```

Expect `200 OK` with body `{"status":"ok"}`, and an `[OK] Navidrome → <target>` log line per currently-configured target.

Now-playing (`listen_type: "playing_now"`):

```bash
curl -i -X POST http://localhost:4567/1/submit-listens \
  -H "Content-Type: application/json" \
  -H "Authorization: Token YOUR_SERVER_WEBHOOK_TOKEN" \
  -d '{
    "listen_type": "playing_now",
    "payload": [{
      "track_metadata": {
        "artist_name": "Test Artist",
        "track_name": "Test Track (now-playing regression check)",
        "release_name": "Test Album",
        "additional_info": { "duration": 200 }
      }
    }]
  }'
```

Expect `200 OK`, and `[REQ] playing_now | Test Artist - Test Track ...` followed by `[NOW]` lines for ListenBrainz/Last.fm (whichever are configured) — Koito should NOT get a now-playing attempt unless `forward_now_playing = true` is explicitly set under `[koito]`.

Below-threshold scrobble (duration 10s — must NOT scrobble anywhere):

```bash
curl -i -X POST http://localhost:4567/1/submit-listens \
  -H "Content-Type: application/json" \
  -H "Authorization: Token YOUR_SERVER_WEBHOOK_TOKEN" \
  -d '{
    "listen_type": "single",
    "payload": [{
      "listened_at": '"$(date +%s)"',
      "track_metadata": {
        "artist_name": "Test Artist",
        "track_name": "Test Track (below threshold, should not scrobble)",
        "additional_info": { "duration": 10 }
      }
    }]
  }'
```

Expect `200 OK`, but **no** `[OK]`/`[FAIL]` log lines at all for this one — it should be silently discarded by the threshold check.

### 2b. Plex

Plex requires `multipart/form-data` with a `payload` field, and the URL-embedded token from your actual `[plex]` config section (use `open` only if `webhook_token` is genuinely unset in your config).

Scrobble:

```bash
curl -i -X POST "http://localhost:4567/webhooks/plex/YOUR_PLEX_TOKEN" \
  -F 'payload={"event":"media.scrobble","Metadata":{"grandparentTitle":"Test Artist","parentTitle":"Test Album","title":"Test Track (plex scrobble regression check)","duration":200000}};type=application/json'
```

Expect `200 OK`, `[OK] Plex → <target>` per configured target.

Now-playing (try both `media.play` and `media.resume`):

```bash
curl -i -X POST "http://localhost:4567/webhooks/plex/YOUR_PLEX_TOKEN" \
  -F 'payload={"event":"media.play","Metadata":{"grandparentTitle":"Test Artist","parentTitle":"Test Album","title":"Test Track (plex now-playing regression check)","duration":200000}};type=application/json'
```

Expect `200 OK`, `[REQ] playing_now (plex) | ...`.

Auth check — wrong token must be rejected:

```bash
curl -i -X POST "http://localhost:4567/webhooks/plex/wrong-token-on-purpose" \
  -F 'payload={"event":"media.scrobble","Metadata":{"title":"should not matter"}};type=application/json'
```

Expect `401 Unauthorized` (only if `[plex].webhook_token` is actually set in your config — if it's unset, any token in the URL is accepted, so skip this specific check).

### 2c. Jellyfin

Uses a plain JSON body and the `X-Scroblin-Token` header (only required if `[jellyfin].webhook_token` is set).

Scrobble (`RunTimeTicks`/`PlaybackPositionTicks` in 100ns units — 200s = 2,000,000,000):

```bash
curl -i -X POST http://localhost:4567/webhooks/jellyfin \
  -H "Content-Type: application/json" \
  -H "X-Scroblin-Token: YOUR_JELLYFIN_TOKEN" \
  -d '{
    "NotificationType": "PlaybackStop",
    "Artist": "Test Artist",
    "Album": "Test Album",
    "Name": "Test Track (jellyfin scrobble regression check)",
    "RunTimeTicks": 2000000000,
    "PlaybackPositionTicks": 2000000000
  }'
```

Expect `200 OK`, `[OK] Jellyfin → <target>` per configured target.

Now-playing:

```bash
curl -i -X POST http://localhost:4567/webhooks/jellyfin \
  -H "Content-Type: application/json" \
  -H "X-Scroblin-Token: YOUR_JELLYFIN_TOKEN" \
  -d '{
    "NotificationType": "PlaybackStart",
    "Artist": "Test Artist",
    "Album": "Test Album",
    "Name": "Test Track (jellyfin now-playing regression check)",
    "RunTimeTicks": 2000000000,
    "PlaybackPositionTicks": 0
  }'
```

Expect `200 OK`, `[REQ] playing_now (jellyfin) | ...`.

Duplicate-stop-at-position-0 filter (must NOT scrobble — this is a pre-existing Jellyfin-specific gotcha, worth reconfirming since `router.rs` was touched by this refactor):

```bash
curl -i -X POST http://localhost:4567/webhooks/jellyfin \
  -H "Content-Type: application/json" \
  -H "X-Scroblin-Token: YOUR_JELLYFIN_TOKEN" \
  -d '{
    "NotificationType": "PlaybackStop",
    "Artist": "Test Artist",
    "Album": "Test Album",
    "Name": "Test Track (position-zero duplicate, should not scrobble)",
    "RunTimeTicks": 2000000000,
    "PlaybackPositionTicks": 0
  }'
```

Expect `200 OK`, no `[OK]`/`[FAIL]` log line.

## Phase 3: Cleanup

- Confirm `conf/config.toml` matches `conf/config.toml.pre-refactor-backup` exactly (diff them) before finishing.
- Remove the test tracks/scrobbles you created from Koito/ListenBrainz/Last.fm if their presence in listen history would be noise you care about (optional — your call).
- Delete `conf/config.toml.pre-refactor-backup` once you've confirmed restoration, or keep it as a known-good fallback.
- `docker compose logs scrobgoblin --tail 200` one more time — confirm no unexpected errors, panics, or restart-loop entries accumulated during testing.

## Rollback plan

If anything in this plan reveals a Critical issue (crash, silent data loss beyond the intentionally-unconfigured-target case, wrong target receiving/not receiving scrobbles):

```bash
git log --oneline | grep 24197fa   # last commit before this refactor
git checkout 24197fa
docker compose build
docker compose up -d
```

Restore `conf/config.toml.pre-refactor-backup` if you'd already modified it for Phase 1 testing — the pre-refactor binary requires all three `[koito]`/`[listenbrainz]`/`[lastfm]` sections to be present, so a config with a section removed will fail to parse on the old binary.

## Reporting back

Summarize, per phase, what passed/failed with the specific log lines or `curl` responses that support each finding — not just "phase 2 passed." If anything deviates from the "expect" blocks above, treat it as a finding worth flagging even if the server didn't crash.
