use crate::{Axis, Layout, PaneId, Rect};

pub const MAX_PANES: usize = 32;
const MIN_WIDTH: u16 = 2;
const MIN_HEIGHT: u16 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Split(Axis),
    Spawned(PaneId),
    SpawnFailed(PaneId),
    Close(PaneId),
    FocusNext,
    FocusPrevious,
    Zoom,
    Resize(Rect),
    Write(Vec<u8>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Spawn { id: PaneId, rows: u16, cols: u16 },
    Close { id: PaneId },
    Resize { id: PaneId, rows: u16, cols: u16 },
    Write { id: PaneId, bytes: Vec<u8> },
    Notice(String),
}
#[derive(Debug)]
struct Pending {
    id: PaneId,
    target: PaneId,
    axis: Axis,
}
#[derive(Debug)]
pub struct Session {
    layout: Option<Layout>,
    area: Rect,
    focus: Option<PaneId>,
    zoomed: bool,
    next_id: PaneId,
    pending: Option<Pending>,
}
impl Session {
    /// The adapter must create pane 1 before constructing the session.
    pub fn new(area: Rect) -> Self {
        Self {
            layout: Some(Layout::Pane(1)),
            area,
            focus: Some(1),
            zoomed: false,
            next_id: 2,
            pending: None,
        }
    }
    pub fn focused(&self) -> Option<PaneId> {
        self.focus
    }
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.layout
            .as_ref()
            .map(|l| {
                l.rectangles(Rect::default())
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn is_zoomed(&self) -> bool {
        self.zoomed
    }
    pub fn rectangles(&self) -> Vec<(PaneId, Rect)> {
        let Some(layout) = &self.layout else {
            return Vec::new();
        };
        let rects = layout.rectangles(self.area);
        if self.zoomed || rects.iter().any(|(_, r)| !fits(*r)) {
            return self
                .focus
                .map(|id| vec![(id, self.area)])
                .unwrap_or_default();
        }
        rects
    }
    fn sizes(&self) -> Vec<Effect> {
        self.rectangles()
            .into_iter()
            .map(|(id, rect)| {
                let content = rect.content();
                Effect::Resize {
                    id,
                    rows: content.height.max(1),
                    cols: content.width.max(1),
                }
            })
            .collect()
    }
    fn cancel(&mut self, effects: &mut Vec<Effect>) {
        if let Some(p) = self.pending.take() {
            effects.push(Effect::Close { id: p.id });
        }
    }
    pub fn apply(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            Event::Split(axis) => {
                if self.pending.is_some() {
                    return vec![Effect::Notice("A split is already pending".into())];
                }
                if self.pane_ids().len() >= MAX_PANES {
                    return vec![Effect::Notice("Pane limit reached (32)".into())];
                }
                let (Some(target), Some(mut proposed)) = (self.focus, self.layout.clone()) else {
                    return effects;
                };
                let id = self.next_id;
                let Some(next) = id.checked_add(1) else {
                    return vec![Effect::Notice("Pane identifiers exhausted".into())];
                };
                proposed.split(target, id, axis);
                let rects = proposed.rectangles(self.area);
                if rects.iter().any(|(_, r)| !fits(*r)) {
                    return vec![Effect::Notice("Terminal is too small to split".into())];
                }
                let Some((_, rect)) = rects.iter().find(|(pane, _)| *pane == id) else {
                    return effects;
                };
                let content = rect.content();
                self.next_id = next;
                self.pending = Some(Pending { id, target, axis });
                effects.push(Effect::Spawn {
                    id,
                    rows: content.height,
                    cols: content.width,
                });
            }
            Event::Spawned(id) => {
                if self.pending.as_ref().is_some_and(|p| p.id == id) {
                    let Some(p) = self.pending.take() else {
                        return effects;
                    };
                    if let Some(layout) = &mut self.layout {
                        if layout.split(p.target, id, p.axis) {
                            self.focus = Some(id);
                            self.zoomed = false;
                            effects.extend(self.sizes());
                        } else {
                            effects.push(Effect::Close { id });
                        }
                    } else {
                        effects.push(Effect::Close { id });
                    }
                } else if !self.pane_ids().contains(&id) {
                    effects.push(Effect::Close { id });
                }
            }
            Event::SpawnFailed(id) => {
                if self.pending.as_ref().is_some_and(|p| p.id == id) {
                    self.pending = None;
                    effects.push(Effect::Notice(format!("Failed to spawn pane {id}")));
                }
                if !self.pane_ids().contains(&id) {
                    effects.push(Effect::Close { id });
                }
            }
            Event::Close(id) => {
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.id == id || p.target == id)
                {
                    self.cancel(&mut effects);
                }
                if self.pane_ids().contains(&id) {
                    self.layout = self.layout.take().and_then(|l| l.remove(id));
                    if self.focus == Some(id) {
                        self.focus = self.pane_ids().first().copied();
                    }
                    if self.layout.is_none() {
                        self.zoomed = false;
                    }
                    effects.push(Effect::Close { id });
                    effects.extend(self.sizes());
                }
            }
            Event::FocusNext | Event::FocusPrevious => {
                let ids = self.pane_ids();
                if let Some(index) = ids.iter().position(|id| Some(*id) == self.focus) {
                    let next = if event == Event::FocusNext {
                        (index + 1) % ids.len()
                    } else {
                        (index + ids.len() - 1) % ids.len()
                    };
                    self.focus = Some(ids[next]);
                    effects.extend(self.sizes());
                }
            }
            Event::Zoom => {
                if self.focus.is_some() {
                    self.zoomed = !self.zoomed;
                    effects.extend(self.sizes());
                }
            }
            Event::Resize(area) => {
                self.cancel(&mut effects);
                self.area = area;
                effects.extend(self.sizes());
            }
            Event::Write(bytes) => {
                if bytes.len() > 65_536 {
                    effects.push(Effect::Notice("Write exceeds 65536 byte limit".into()));
                } else if let Some(id) = self.focus {
                    effects.push(Effect::Write { id, bytes });
                }
            }
        }
        effects
    }
}
fn fits(rect: Rect) -> bool {
    rect.width >= MIN_WIDTH && rect.height >= MIN_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area() -> Rect {
        Rect {
            width: 160,
            height: 80,
            ..Rect::default()
        }
    }
    fn spawn_id(effects: &[Effect]) -> PaneId {
        match effects[0] {
            Effect::Spawn { id, .. } => id,
            ref effect => panic!("expected spawn, got {effect:?}"),
        }
    }
    #[test]
    fn transactional_cancel_and_stale_cleanup() {
        let mut s = Session::new(area());
        let id = spawn_id(&s.apply(Event::Split(Axis::Columns)));
        assert_eq!(s.pane_ids(), vec![1]);
        assert_eq!(s.focused(), Some(1));
        assert!(
            s.apply(Event::Resize(area()))
                .contains(&Effect::Close { id })
        );
        assert_eq!(s.apply(Event::Spawned(id)), vec![Effect::Close { id }]);
        let next = spawn_id(&s.apply(Event::Split(Axis::Rows)));
        assert!(next > id);
        s.apply(Event::SpawnFailed(next));
        assert_eq!(s.pane_ids(), vec![1]);
        assert_eq!(
            s.apply(Event::Spawned(next)),
            vec![Effect::Close { id: next }]
        );
    }
    #[test]
    fn commit_focus_zoom_and_tiny_fallback() {
        let mut s = Session::new(area());
        let id = spawn_id(&s.apply(Event::Split(Axis::Columns)));
        s.apply(Event::Spawned(id));
        assert_eq!(s.pane_ids(), vec![1, id]);
        s.apply(Event::FocusPrevious);
        assert_eq!(s.focused(), Some(1));
        s.apply(Event::FocusNext);
        assert_eq!(s.focused(), Some(id));
        s.apply(Event::Zoom);
        assert_eq!(s.rectangles(), vec![(id, area())]);
        s.apply(Event::Zoom);
        s.apply(Event::Resize(Rect::default()));
        assert_eq!(s.rectangles(), vec![(id, Rect::default())]);
        s.apply(Event::Close(id));
        s.apply(Event::Close(1));
        assert!(s.rectangles().is_empty());
        assert_eq!(s.focused(), None);
        assert!(s.apply(Event::Split(Axis::Rows)).is_empty());
    }
    #[test]
    fn pending_target_close_and_capacity() {
        let mut s = Session::new(Rect {
            width: u16::MAX,
            height: u16::MAX,
            ..Rect::default()
        });
        let id = spawn_id(&s.apply(Event::Split(Axis::Rows)));
        assert!(s.apply(Event::Close(1)).contains(&Effect::Close { id }));
        assert_eq!(s.apply(Event::Spawned(id)), vec![Effect::Close { id }]);
        let mut s = Session::new(Rect {
            width: u16::MAX,
            height: u16::MAX,
            ..Rect::default()
        });
        for i in 0..31 {
            let target = s
                .rectangles()
                .into_iter()
                .max_by_key(|(_, r)| u32::from(r.width) * u32::from(r.height))
                .unwrap()
                .0;
            while s.focused() != Some(target) {
                s.apply(Event::FocusNext);
            }
            let id = spawn_id(&s.apply(Event::Split(if i % 2 == 0 {
                Axis::Rows
            } else {
                Axis::Columns
            })));
            s.apply(Event::Spawned(id));
        }
        assert_eq!(s.pane_ids().len(), 32);
        assert!(matches!(
            s.apply(Event::Split(Axis::Rows))[0],
            Effect::Notice(_)
        ));
    }
}
