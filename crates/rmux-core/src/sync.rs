//! Latest-frame session sync. No clocks, sockets, or terminal contents.

pub const HARD_FRAME_LIMIT: usize = 262_144;
pub const HARD_PREDICTION_LIMIT: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionKey(pub [u8; 32]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub epoch: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observe {
    Applied,
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncFault {
    EmptyEpoch,
    TooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketFault {
    BadKey,
    TooLarge,
}

#[derive(Debug, Default)]
pub struct ScreenSync {
    latest: Option<Snapshot>,
    dropped: u64,
}

impl ScreenSync {
    pub fn publish(
        &mut self,
        epoch: u64,
        bytes: &[u8],
        limit: usize,
    ) -> Result<Snapshot, SyncFault> {
        if epoch == 0 {
            return Err(SyncFault::EmptyEpoch);
        }
        if bytes.len() > limit || bytes.len() > HARD_FRAME_LIMIT {
            return Err(SyncFault::TooLarge);
        }
        let snapshot = Snapshot {
            epoch,
            bytes: bytes.to_vec(),
        };
        if self
            .latest
            .as_ref()
            .is_some_and(|current| current.epoch >= epoch)
        {
            self.dropped = self.dropped.saturating_add(1);
            return Ok(snapshot);
        }
        self.latest = Some(snapshot.clone());
        Ok(snapshot)
    }

    pub fn observe(&mut self, snapshot: Snapshot) -> Observe {
        if self
            .latest
            .as_ref()
            .is_some_and(|current| current.epoch >= snapshot.epoch)
        {
            self.dropped = self.dropped.saturating_add(1);
            return Observe::Stale;
        }
        self.latest = Some(snapshot);
        Observe::Applied
    }

    pub fn latest(&self) -> Option<&Snapshot> {
        self.latest.as_ref()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

fn keys_match(expected: &SessionKey, presented: &SessionKey) -> bool {
    let mut diff = 0u8;
    for (left, right) in expected.0.iter().zip(presented.0) {
        diff |= left ^ right;
    }
    diff == 0
}

pub fn open_packet<'a>(
    expected: &SessionKey,
    presented: &SessionKey,
    body: &'a [u8],
    limit: usize,
) -> Result<&'a [u8], PacketFault> {
    if !keys_match(expected, presented) {
        return Err(PacketFault::BadKey);
    }
    if body.len() > limit || body.len() > HARD_FRAME_LIMIT {
        return Err(PacketFault::TooLarge);
    }
    Ok(body)
}

#[derive(Debug)]
pub struct EchoPrediction {
    pending: Vec<u8>,
    limit: usize,
    corrections: u64,
}

impl EchoPrediction {
    pub fn new(limit: usize) -> Self {
        Self {
            pending: Vec::new(),
            limit: limit.min(HARD_PREDICTION_LIMIT),
            corrections: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > self.limit || self.pending.len() + bytes.len() > self.limit {
            self.rollback();
            return false;
        }
        self.pending.extend_from_slice(bytes);
        true
    }

    pub fn confirm(&mut self, echoed: &[u8]) {
        let matched = self
            .pending
            .iter()
            .zip(echoed)
            .take_while(|(pending, echoed)| pending == echoed)
            .count();
        if matched == 0 && !echoed.is_empty() && !self.pending.is_empty() {
            self.rollback();
            return;
        }
        self.pending.drain(..matched);
    }

    pub fn rollback(&mut self) -> Vec<u8> {
        if !self.pending.is_empty() {
            self.corrections = self.corrections.saturating_add(1);
        }
        std::mem::take(&mut self.pending)
    }

    pub fn overlay(&self) -> &[u8] {
        &self.pending
    }

    pub fn corrections(&self) -> u64 {
        self.corrections
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_epoch_replaces_older() {
        let mut sync = ScreenSync::default();
        sync.publish(1, b"one", 32).unwrap();
        sync.publish(2, b"two", 32).unwrap();
        assert_eq!(sync.latest().unwrap().bytes, b"two");
    }

    #[test]
    fn late_epoch_is_stale_and_gap_keeps_latest() {
        let mut sync = ScreenSync::default();
        sync.publish(1, b"one", 32).unwrap();
        sync.publish(4, b"four", 32).unwrap();
        assert_eq!(
            sync.observe(Snapshot {
                epoch: 2,
                bytes: b"two".to_vec(),
            }),
            Observe::Stale
        );
        assert_eq!(sync.latest().unwrap().bytes, b"four");
        assert_eq!(sync.dropped(), 1);
    }

    #[test]
    fn bad_key_drops_packet() {
        let expected = SessionKey([1; 32]);
        let wrong = SessionKey([2; 32]);
        assert_eq!(
            open_packet(&expected, &wrong, b"frame", 32),
            Err(PacketFault::BadKey)
        );
        assert_eq!(
            open_packet(&expected, &expected, b"frame", 32),
            Ok(&b"frame"[..])
        );
    }

    #[test]
    fn echo_confirms_then_rolls_back() {
        let mut echo = EchoPrediction::new(8);
        assert!(echo.push(b"ab"));
        echo.confirm(b"a");
        assert_eq!(echo.overlay(), b"b");
        echo.confirm(b"z");
        assert!(echo.overlay().is_empty());
        assert_eq!(echo.corrections(), 1);
    }
}
