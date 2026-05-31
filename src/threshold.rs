use crate::event::PlayEvent;

pub fn qualifies(event: &PlayEvent) -> bool {
    match event.duration_secs {
        Some(d) => d >= 30,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Source;
    use chrono::Utc;

    fn make_event(duration_secs: Option<u64>) -> PlayEvent {
        PlayEvent {
            artist: "Artist".to_string(),
            album: None,
            track: "Track".to_string(),
            duration_secs,
            played_at: Utc::now(),
            source: Source::Navidrome,
        }
    }

    #[test]
    fn qualifies_when_duration_missing() {
        assert!(qualifies(&make_event(None)));
    }

    #[test]
    fn qualifies_when_duration_30_or_more() {
        assert!(qualifies(&make_event(Some(30))));
        assert!(qualifies(&make_event(Some(300))));
    }

    #[test]
    fn disqualifies_when_duration_under_30() {
        assert!(!qualifies(&make_event(Some(29))));
        assert!(!qualifies(&make_event(Some(0))));
    }
}
