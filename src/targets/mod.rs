pub mod koito;
pub mod lastfm;
pub mod listenbrainz;

use crate::{config::Config, event::PlayEvent};
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
