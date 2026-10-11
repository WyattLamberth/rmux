//! Production state transitions, independent of processes, clocks and scheduling.
use crate::layout::{Axis, Layout, Rect};
use std::collections::BTreeMap;

pub const MAX_PANES: usize = 16;
pub const MAX_ROWS: u16 = 200;
pub const MAX_COLS: u16 = 500;
pub const SCROLLBACK_LINES: usize = 1000;
pub const MAX_INPUT: usize = 16 * 1024;
pub const OUTPUT_CHUNK: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PaneId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Starting,
    Running,
    Exited(u32),
    Failed,
}

pub struct Pane {
    pub lifecycle: Lifecycle,
    pub terminal: crate::terminal::Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Split(Axis),
    FocusNext,
    ToggleZoom,
    Close,
    Resize { cols: u16, rows: u16 },
    Spawned(PaneId),
    Failed(PaneId),
    Exited(PaneId, u32),
    Output(PaneId, Vec<u8>),
    Input(Vec<u8>),
    Scroll(i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Spawn(PaneId, Rect),
    Resize(PaneId, Rect),
    Write(PaneId, Vec<u8>),
    Stop(PaneId),
}

pub struct Session {
    panes: BTreeMap<PaneId, Pane>,
    layout: Option<Layout>,
    focus: Option<PaneId>,
    zoom: bool,
    next_id: u64,
    area: Rect,
}

impl Session {
    pub fn new(cols: u16, rows: u16) -> (Self, Vec<Effect>) {
        let mut session = Self {
            panes: BTreeMap::new(),
            layout: None,
            focus: None,
            zoom: false,
            next_id: 0,
            area: Self::area(cols, rows),
        };
        let effects = session.add_pane(Axis::Columns);
        (session, effects)
    }

    fn area(cols: u16, rows: u16) -> Rect {
        Rect {
            cols: cols.min(MAX_COLS),
            rows: rows.saturating_sub(1).min(MAX_ROWS),
            ..Rect::default()
        }
    }

    pub fn panes(&self) -> &BTreeMap<PaneId, Pane> {
        &self.panes
    }
    pub fn focus(&self) -> Option<PaneId> {
        self.focus
    }
    pub fn zoomed(&self) -> bool {
        self.zoom
    }
    pub fn finished(&self) -> bool {
        self.panes
            .values()
            .all(|p| matches!(p.lifecycle, Lifecycle::Exited(_) | Lifecycle::Failed))
    }

    pub fn rectangles(&self) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::with_capacity(self.panes.len());
        if self.zoom {
            for id in self.panes.keys() {
                out.push((
                    *id,
                    if Some(*id) == self.focus {
                        self.area
                    } else {
                        Rect::default()
                    },
                ));
            }
        } else if let Some(layout) = &self.layout {
            layout.rectangles(self.area, &mut out);
        }
        out
    }

    fn resize_effects(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        for (id, rect) in self.rectangles() {
            // Hidden panes retain their last real size, so shells never receive 0x0.
            if rect.visible() {
                if let Some(pane) = self.panes.get_mut(&id) {
                    if pane.terminal.size() != (rect.rows, rect.cols) {
                        pane.terminal.resize(rect.rows, rect.cols);
                        if pane.lifecycle == Lifecycle::Running {
                            effects.push(Effect::Resize(id, rect));
                        }
                    }
                }
            }
        }
        effects
    }

    fn add_pane(&mut self, axis: Axis) -> Vec<Effect> {
        if self.panes.len() >= MAX_PANES {
            return Vec::new();
        }
        let Some(next_id) = self.next_id.checked_add(1) else {
            return Vec::new();
        };
        let id = PaneId(self.next_id);
        self.next_id = next_id;
        if let (Some(layout), Some(focus)) = (&mut self.layout, self.focus) {
            layout.split(focus, id, axis);
        } else {
            self.layout = Some(Layout::Leaf(id));
        }
        self.focus = Some(id);
        self.panes.insert(
            id,
            Pane {
                lifecycle: Lifecycle::Starting,
                terminal: crate::terminal::Terminal::new(1, 1, SCROLLBACK_LINES),
            },
        );
        let mut effects = self.resize_effects();
        let rect = self
            .rectangles()
            .into_iter()
            .find(|(p, _)| *p == id)
            .map(|(_, r)| r)
            .unwrap_or_default();
        effects.push(Effect::Spawn(
            id,
            Rect {
                cols: rect.cols.max(1),
                rows: rect.rows.max(1),
                ..rect
            },
        ));
        effects
    }

    pub fn apply(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Split(axis) => return self.add_pane(axis),
            Event::FocusNext => {
                self.focus = self
                    .panes
                    .keys()
                    .copied()
                    .find(|id| Some(*id) > self.focus)
                    .or_else(|| self.panes.keys().next().copied());
                if self.zoom {
                    return self.resize_effects();
                }
            }
            Event::ToggleZoom => {
                self.zoom = !self.zoom;
                return self.resize_effects();
            }
            Event::Resize { cols, rows } => {
                self.area = Self::area(cols, rows);
                return self.resize_effects();
            }
            Event::Close => {
                if let Some(id) = self.focus.take() {
                    self.panes.remove(&id);
                    self.layout = self.layout.take().and_then(|layout| layout.remove(id));
                    self.focus = self.panes.keys().next().copied();
                    let mut effects = vec![Effect::Stop(id)];
                    effects.extend(self.resize_effects());
                    return effects;
                }
            }
            Event::Spawned(id) => {
                if let Some(pane) = self.panes.get_mut(&id) {
                    if pane.lifecycle == Lifecycle::Starting {
                        pane.lifecycle = Lifecycle::Running;
                    }
                }
            }
            Event::Failed(id) | Event::Exited(id, _) => {
                if let Some(pane) = self.panes.get_mut(&id) {
                    if matches!(pane.lifecycle, Lifecycle::Starting | Lifecycle::Running) {
                        pane.lifecycle = match event {
                            Event::Exited(_, code) => Lifecycle::Exited(code),
                            _ => Lifecycle::Failed,
                        };
                        return vec![Effect::Stop(id)];
                    }
                }
            }
            Event::Output(id, bytes) => {
                if let Some(pane) = self.panes.get_mut(&id) {
                    if pane.lifecycle == Lifecycle::Running {
                        pane.terminal
                            .process(&bytes[..bytes.len().min(OUTPUT_CHUNK)]);
                    }
                }
            }
            Event::Input(bytes) => {
                if bytes.len() <= MAX_INPUT {
                    if let Some(id) = self.focus {
                        if let Some(pane) = self.panes.get_mut(&id) {
                            if pane.lifecycle == Lifecycle::Running {
                                pane.terminal.screen_mut().set_scrollback(0);
                                return vec![Effect::Write(id, bytes)];
                            }
                        }
                    }
                }
            }
            Event::Scroll(delta) => {
                if let Some(pane) = self.focus.and_then(|id| self.panes.get_mut(&id)) {
                    let offset = pane
                        .terminal
                        .screen()
                        .scrollback()
                        .saturating_add_signed(delta as isize)
                        .min(SCROLLBACK_LINES);
                    pane.terminal.screen_mut().set_scrollback(offset);
                }
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_lifecycle_ignores_stale_events() {
        let (mut s, _) = Session::new(80, 24);
        s.apply(Event::Spawned(PaneId(0)));
        assert_eq!(
            s.apply(Event::Failed(PaneId(0))),
            vec![Effect::Stop(PaneId(0))]
        );
        s.apply(Event::Spawned(PaneId(0)));
        s.apply(Event::Output(PaneId(0), b"stale".to_vec()));
        assert_eq!(s.panes()[&PaneId(0)].lifecycle, Lifecycle::Failed);
        assert!(
            s.panes()[&PaneId(0)]
                .terminal
                .screen()
                .contents()
                .is_empty()
        );
        assert!(s.apply(Event::Input(vec![1])).is_empty());
        assert!(s.finished());
    }

    #[test]
    fn splits_zoom_focus_close_and_limits() {
        let (mut s, _) = Session::new(80, 24);
        s.apply(Event::Spawned(PaneId(0)));
        s.apply(Event::Split(Axis::Columns));
        s.apply(Event::Split(Axis::Rows));
        assert_eq!(s.rectangles().len(), 3);
        s.apply(Event::ToggleZoom);
        assert_eq!(
            s.rectangles().iter().filter(|(_, r)| r.visible()).count(),
            1
        );
        s.apply(Event::FocusNext);
        assert_eq!(s.focus(), Some(PaneId(0)));
        s.apply(Event::Close);
        assert_eq!(s.focus(), Some(PaneId(1)));
        for _ in 0..30 {
            s.apply(Event::Split(Axis::Rows));
        }
        assert_eq!(s.panes().len(), MAX_PANES);
        for cols in 0..4 {
            for rows in 0..4 {
                s.apply(Event::Resize { cols, rows });
                for pane in s.panes().values() {
                    let (r, c) = pane.terminal.size();
                    assert!(r > 0 && c > 0);
                }
            }
        }
    }

    #[test]
    fn output_history_and_input_are_bounded() {
        let (mut s, _) = Session::new(20, 4);
        s.apply(Event::Spawned(PaneId(0)));
        for _ in 0..3000 {
            s.apply(Event::Output(PaneId(0), b"line\r\n".to_vec()));
        }
        s.apply(Event::Scroll(i32::MAX));
        assert!(s.panes()[&PaneId(0)].terminal.screen().scrollback() <= SCROLLBACK_LINES);
        assert!(s.apply(Event::Input(vec![0; MAX_INPUT + 1])).is_empty());
        s.apply(Event::Input(b"x".to_vec()));
        assert_eq!(s.panes()[&PaneId(0)].terminal.screen().scrollback(), 0);
    }
}
