pub mod koito;
pub mod lastfm;
pub mod listenbrainz;

use crate::{config::Config, event::{NowPlayingEvent, PlayEvent}};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

pub async fn fan_out(cfg: Arc<Config>, client: reqwest::Client, event: PlayEvent) {
    let event = Arc::new(event);

    let (cfg1, client1, event1) = (cfg.clone(), client.clone(), event.clone());
    let t1 = tokio::spawn(async move {
        for attempt in 1..=3u32 {
            match koito::submit(&cfg1.koito, &client1, &event1).await {
                Ok(()) => {
                    println!("[OK] {} → Koito | {} - {}", event1.source, event1.artist, event1.track);
                    break;
                }
                Err(e) => retry_log("Koito", &event1, attempt, &e).await,
            }
        }
    });

    let (cfg2, client2, event2) = (cfg.clone(), client.clone(), event.clone());
    let t2 = tokio::spawn(async move {
        for attempt in 1..=3u32 {
            match listenbrainz::submit(&cfg2.listenbrainz, &client2, &event2).await {
                Ok(()) => {
                    println!("[OK] {} → ListenBrainz | {} - {}", event2.source, event2.artist, event2.track);
                    break;
                }
                Err(e) => retry_log("ListenBrainz", &event2, attempt, &e).await,
            }
        }
    });

    let (cfg3, client3, event3) = (cfg.clone(), client.clone(), event.clone());
    let t3 = tokio::spawn(async move {
        for attempt in 1..=3u32 {
            match lastfm::submit(&cfg3.lastfm, &client3, &event3).await {
                Ok(()) => {
                    println!("[OK] {} → Last.fm | {} - {}", event3.source, event3.artist, event3.track);
                    break;
                }
                Err(e) => retry_log("Last.fm", &event3, attempt, &e).await,
            }
        }
    });

    let _ = tokio::join!(t1, t2, t3);
}

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

async fn retry_log(target: &str, event: &PlayEvent, attempt: u32, e: &anyhow::Error) {
    let delays = [1u64, 4];
    if attempt < 3 {
        let delay = delays[(attempt - 1) as usize];
        eprintln!(
            "[FAIL] {} → {} | {} - {} | attempt {}/3 | {} | retrying in {}s",
            event.source, target, event.artist, event.track, attempt, e, delay
        );
        sleep(Duration::from_secs(delay)).await;
    } else {
        eprintln!(
            "[FAIL] {} → {} | {} - {} | attempt 3/3 | {}",
            event.source, target, event.artist, event.track, e
        );
    }
}

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
