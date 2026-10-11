use crate::{Axis, Layout, PaneId, PaneLayout, Rect};

pub const MAX_PANES: usize = 32;
pub const HARD_MAX_PANES: usize = MAX_PANES;
pub const HARD_WRITE_LIMIT: usize = 65_536;
const MIN_WIDTH: u16 = 2;
const MIN_HEIGHT: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_panes: usize,
    pub write_limit: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_panes: HARD_MAX_PANES,
            write_limit: HARD_WRITE_LIMIT,
        }
    }
}

impl Limits {
    pub fn new(max_panes: usize, write_limit: usize) -> Result<Self, &'static str> {
        if !(1..=HARD_MAX_PANES).contains(&max_panes) {
            return Err("max_panes out of range");
        }
        if write_limit == 0 || write_limit > HARD_WRITE_LIMIT {
            return Err("write_limit out of range");
        }
        Ok(Self {
            max_panes,
            write_limit,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Split(Axis),
    Spawned(PaneId),
    SpawnFailed(PaneId),
    Close(PaneId),
    /// Drop the display. Shells stay open.
    Detach,
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
    limits: Limits,
    pane_layout: PaneLayout,
}
impl Session {
    /// The adapter must create pane 1 before constructing the session.
    pub fn new(area: Rect) -> Self {
        Self::with_limits(area, Limits::default())
    }

    pub fn with_limits(area: Rect, limits: Limits) -> Self {
        Self::with_options(area, limits, PaneLayout::default())
    }

    pub fn with_options(area: Rect, limits: Limits, pane_layout: PaneLayout) -> Self {
        Self {
            layout: Some(Layout::Pane(1)),
            area,
            focus: Some(1),
            zoomed: false,
            next_id: 2,
            pending: None,
            limits,
            pane_layout,
        }
    }
    pub fn focused(&self) -> Option<PaneId> {
        self.focus
    }
    pub fn pane_layout(&self) -> PaneLayout {
        self.pane_layout
    }
    pub fn content_rect(&self, rect: Rect) -> Rect {
        self.pane_layout.content(rect)
    }
    fn fits(&self, rect: Rect) -> bool {
        let content = self.content_rect(rect);
        content.width >= MIN_WIDTH && content.height >= MIN_HEIGHT
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
        if self.zoomed || rects.iter().any(|(_, r)| !self.fits(*r)) {
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
                let content = self.content_rect(rect);
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
                if self.pane_ids().len() >= self.limits.max_panes {
                    return vec![Effect::Notice(format!(
                        "Pane limit reached ({})",
                        self.limits.max_panes
                    ))];
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
                if rects.iter().any(|(_, r)| !self.fits(*r)) {
                    return vec![Effect::Notice("Terminal is too small to split".into())];
                }
                let Some((_, rect)) = rects.iter().find(|(pane, _)| *pane == id) else {
                    return effects;
                };
                let content = self.content_rect(*rect);
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
            Event::Detach => {}
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
                if bytes.len() > self.limits.write_limit {
                    effects.push(Effect::Notice(format!(
                        "Write exceeds {} byte limit",
                        self.limits.write_limit
                    )));
                } else if let Some(id) = self.focus {
                    effects.push(Effect::Write { id, bytes });
                }
            }
        }
        effects
    }
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
    #[test]
    fn configured_content_sizes_follow_nested_splits_zoom_and_resize() {
        for headers in [false, true] {
            for padding in [0, 1, 16] {
                let pane_layout = PaneLayout::new(headers, padding).unwrap();
                let mut s = Session::with_options(area(), Limits::default(), pane_layout);
                for axis in [Axis::Columns, Axis::Rows] {
                    let spawn = s.apply(Event::Split(axis));
                    if let Some(Effect::Spawn { id, rows, cols }) = spawn.first() {
                        let (id, rows, cols) = (*id, *rows, *cols);
                        let sizes = s.apply(Event::Spawned(id));
                        let rect = s
                            .rectangles()
                            .into_iter()
                            .find(|(pane, _)| *pane == id)
                            .unwrap()
                            .1;
                        let content = s.content_rect(rect);
                        assert_eq!((rows, cols), (content.height, content.width));
                        assert!(sizes.contains(&Effect::Resize { id, rows, cols }));
                    }
                }
                for event in [Event::Zoom, Event::Zoom, Event::FocusPrevious] {
                    let effects = s.apply(event);
                    for (id, rect) in s.rectangles() {
                        let content = s.content_rect(rect);
                        assert!(effects.contains(&Effect::Resize {
                            id,
                            rows: content.height.max(1),
                            cols: content.width.max(1),
                        }));
                    }
                }
                for width in 0..8 {
                    for height in 0..8 {
                        let effects = s.apply(Event::Resize(Rect {
                            width,
                            height,
                            ..Rect::default()
                        }));
                        for (id, rect) in s.rectangles() {
                            let content = s.content_rect(rect);
                            assert!(content.x <= width && content.y <= height);
                            assert!(content.x + content.width <= width);
                            assert!(content.y + content.height <= height);
                            assert!(effects.contains(&Effect::Resize {
                                id,
                                rows: content.height.max(1),
                                cols: content.width.max(1),
                            }));
                        }
                    }
                }
            }
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
