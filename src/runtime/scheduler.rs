#[derive(Debug, Clone)]
pub struct Scheduler {
    next_batch_id: u64,
    next_event_sequence: u64,
    next_timestamp_ms: u64,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            next_batch_id: 1,
            next_event_sequence: 1,
            next_timestamp_ms: 1,
        }
    }
}

impl Scheduler {
    pub fn next_batch_id(&mut self) -> u64 {
        let id = self.next_batch_id;
        self.next_batch_id += 1;
        id
    }

    pub fn next_event_sequence(&mut self) -> u64 {
        let sequence = self.next_event_sequence;
        self.next_event_sequence += 1;
        sequence
    }

    pub fn next_timestamp_ms(&mut self) -> u64 {
        let timestamp = self.next_timestamp_ms;
        self.next_timestamp_ms += 1;
        timestamp
    }
}
