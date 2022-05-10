
use tokio::sync::broadcast;

#[derive(Clone)]
pub enum Event {
    StartProviding {
        key: String,
    },
    StopProviding {
        key: String,
    },
    Put {
        key: String,
        value: String,
    },
    PeerReady,
    Simple(String)
}

#[derive(Debug)]
pub struct EventBus {
    pub sender: broadcast::Sender<Event>,
    pub receiver: broadcast::Receiver<Event>,
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        EventBus {
            sender: self.sender.clone(),
            receiver: self.sender.subscribe()
        }
    }
}

impl EventBus {

    pub fn new() -> Self {
        let (tx, mut rx) = broadcast::channel(16);
        EventBus {
            sender: tx,
            receiver: rx,
        }
    }

}
