use std::sync::mpsc;
use std::sync::{Arc, Mutex};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Event {
    RepositoryAdded {
        repository_id: i64,
        name: String,
    },
    RepositoryRemoved {
        repository_id: i64,
    },
    TrackDiscovered {
        repository_id: i64,
        track: crate::models::Track,
    },
    TrackDownloaded {
        track_id: i64,
        path: std::path::PathBuf,
    },
    PlaybackStarted {
        track_id: i64,
    },
    PlaybackPaused {
        track_id: i64,
    },
    PlaybackStopped {
        track_id: i64,
    },
    VolumeChanged {
        volume: f32,
    },
    Error {
        error: String,
    },
}

pub struct EventBus {
    subscribers: Arc<Mutex<Vec<mpsc::Sender<Event>>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[allow(dead_code)]
    pub fn publish(&self, event: Event) {
        let subscribers = self.subscribers.lock().unwrap();
        for subscriber in subscribers.iter() {
            let _ = subscriber.send(event.clone());
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        Self {
            subscribers: self.subscribers.clone(),
        }
    }
}
