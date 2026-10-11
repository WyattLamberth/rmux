use crate::{Effect, Event, PaneId, Session};
use std::{collections::VecDeque, fmt::Display};

/// Synchronous application port. Async adapters may drive Session effects directly.
/// Close must be idempotent and attempt cleanup even after a partial spawn failure.
pub trait PaneHost {
    type Error: Display;
    fn spawn(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<(), Self::Error>;
    fn close(&mut self, id: PaneId) -> Result<(), Self::Error>;
    fn resize(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<(), Self::Error>;
    fn write(&mut self, id: PaneId, bytes: &[u8]) -> Result<(), Self::Error>;
}

pub struct Controller<H: PaneHost> {
    pub session: Session,
    pub host: H,
    pub notices: Vec<String>,
}
impl<H: PaneHost> Controller<H> {
    pub fn new(session: Session, host: H) -> Self {
        Self {
            session,
            host,
            notices: Vec::new(),
        }
    }
    fn notice(&mut self, message: String) {
        if self.notices.len() >= 32 {
            self.notices.remove(0);
        }
        self.notices.push(message);
    }
    /// Resolve failures through the same state transitions used by external events.
    /// No recursive calls; one dispatch processes at most 1024 events.
    pub fn dispatch(&mut self, event: Event) {
        let mut queue = VecDeque::from([event]);
        for _ in 0..1024 {
            let Some(event) = queue.pop_front() else {
                return;
            };
            for effect in self.session.apply(event) {
                match effect {
                    Effect::Spawn { id, rows, cols } => match self.host.spawn(id, rows, cols) {
                        Ok(()) => queue.push_back(Event::Spawned(id)),
                        Err(error) => {
                            self.notice(format!("Spawn pane {id}: {error}"));
                            queue.push_back(Event::SpawnFailed(id));
                        }
                    },
                    Effect::Close { id } => {
                        if let Err(error) = self.host.close(id) {
                            self.notice(format!("Close pane {id}: {error}"));
                        }
                    }
                    Effect::Resize { id, rows, cols } => {
                        if let Err(error) = self.host.resize(id, rows, cols) {
                            self.notice(format!("Resize pane {id}: {error}"));
                            queue.push_back(Event::Close(id));
                        }
                    }
                    Effect::Write { id, bytes } => {
                        if let Err(error) = self.host.write(id, &bytes) {
                            self.notice(format!("Write pane {id}: {error}"));
                        }
                    }
                    Effect::Notice(message) => self.notice(message),
                }
            }
        }
        self.notice("Event work limit reached".into());
    }
}
