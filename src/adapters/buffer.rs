use std::{
    collections::VecDeque,
    io,
    sync::{Condvar, Mutex},
};

const CAPACITY: usize = 1024 * 1024;
const BATCH_SIZE: usize = 4096;

#[derive(Default)]
struct State {
    bytes: VecDeque<u8>,
    closed: bool,
}

/// Byte-bounded input, so a burst of single-key events does not exhaust a
/// message-count queue and silently corrupt the command being typed.
#[derive(Default)]
pub(crate) struct InputQueue {
    state: Mutex<State>,
    ready: Condvar,
}

impl InputQueue {
    pub fn try_send(&self, bytes: Vec<u8>) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("input queue lock poisoned"))?;
        if state.closed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "pane input writer closed",
            ));
        }
        if bytes.len() > CAPACITY.saturating_sub(state.bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "pane input byte buffer is full",
            ));
        }
        state.bytes.extend(bytes);
        self.ready.notify_one();
        Ok(())
    }

    pub fn take(&self) -> io::Result<Option<Vec<u8>>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("input queue lock poisoned"))?;
        while state.bytes.is_empty() && !state.closed {
            state = self
                .ready
                .wait(state)
                .map_err(|_| io::Error::other("input queue lock poisoned"))?;
        }
        if state.closed {
            return Ok(None);
        }
        let count = state.bytes.len().min(BATCH_SIZE);
        Ok(Some(state.bytes.drain(..count).collect()))
    }

    pub fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.bytes.clear();
        }
        self.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_keystrokes_coalesce_without_loss_or_reordering() {
        let queue = InputQueue::default();
        let command: Vec<_> = (0..10_000).map(|index| (index % 251) as u8).collect();
        for &byte in &command {
            queue.try_send(vec![byte]).unwrap();
        }
        let mut delivered = Vec::new();
        while delivered.len() < command.len() {
            delivered.extend(queue.take().unwrap().unwrap());
        }
        assert_eq!(delivered, command);
        queue.close();
        assert!(queue.take().unwrap().is_none());
        assert_eq!(
            queue.try_send(vec![1]).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn rejected_batch_preserves_previously_accepted_bytes() {
        let queue = InputQueue::default();
        queue.try_send(vec![b'a'; CAPACITY]).unwrap();
        assert_eq!(
            queue.try_send(vec![b'b']).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        let mut received = 0;
        while received < CAPACITY {
            let bytes = queue.take().unwrap().unwrap();
            assert!(bytes.iter().all(|byte| *byte == b'a'));
            received += bytes.len();
        }
        assert_eq!(received, CAPACITY);
    }
}
