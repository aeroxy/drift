use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::protocol::messages::TransferEntry;

#[allow(dead_code)]
#[derive(Clone)]
pub struct PendingTransfer {
    pub id: Uuid,
    pub entries: Vec<TransferEntry>,
}

#[allow(dead_code)]
pub struct TransferManager {
    pending: Arc<Mutex<HashMap<Uuid, PendingTransfer>>>,
}

impl TransferManager {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn register(&self, transfer: PendingTransfer) {
        self.pending.lock().await.insert(transfer.id, transfer);
    }

    pub async fn get(&self, id: &Uuid) -> Option<PendingTransfer> {
        self.pending.lock().await.get(id).cloned()
    }

    pub async fn remove(&self, id: &Uuid) {
        self.pending.lock().await.remove(id);
    }
}
