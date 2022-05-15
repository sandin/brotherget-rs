
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum Event {
    // peer events
    PeerStarted {
        peer_id: String,
        addr: String,
        port: u32,
    },
    PeerStoped,

    // proxy events
    ProxyStarted {
        addr: String,
        port: u32,
    },
    ProxyStoped,

    // kad events
    StartProviding {
        key: String,
    },
    StopProviding {
        key: String,
    },
    GetProviders {
        key: String,
    },
    GetProvidersResult {
        key: String,
        providers: Vec<String>, // peer ids
    },
    PutRecord {
        key: String,
        value: Vec<u8>,
    },
    PutRecordResult {
        success: bool,
    },
    GetRecord {
        key: String,
    },
    GetRecordResult {
        key: String,
        value: Vec<u8>,
    },

    // download events
    DownloadSuccessed {
        filename: String,
    },
    DownloadFailed {
        error: String,
    },
}

/// There are many asynchronous components that need to communicate with each other,
/// we use tokio broadcast as event bus to fulfill this need, 
/// maybe it's not perfect, but it's works for now.
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
        let (tx, rx) = broadcast::channel(32);
        EventBus {
            sender: tx,
            receiver: rx,
        }
    }

}
