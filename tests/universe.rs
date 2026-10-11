//! Replay with RMUX_TEST_SEED and RMUX_TEST_STEPS. No OS randomness or wall clock.
use rmux_core::{Axis, Controller, Effect, Event, Layout, Limits, PaneHost, PaneId, Rect, Session};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Default)]
struct Host {
    live: BTreeMap<PaneId, (u16, u16)>,
    fail: u8,
    largest: PaneId,
    output: VecDeque<(u64, PaneId, Vec<u8>)>,
    clock: u64,
}
impl Host {
    fn initial() -> Self {
        Self {
            live: BTreeMap::from([(1, (39, 120))]),
            largest: 1,
            ..Self::default()
        }
    }
}
impl PaneHost for Host {
    type Error = &'static str;
    fn spawn(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<(), Self::Error> {
        assert!(id > self.largest, "reused spawn identifier");
        self.largest = id;
        self.live.insert(id, (rows, cols)); // Include partially successful spawn failures.
        if self.fail == 1 {
            Err("partial spawn failure")
        } else {
            Ok(())
        }
    }
    fn close(&mut self, id: PaneId) -> Result<(), Self::Error> {
        self.live.remove(&id);
        if self.fail == 4 {
            Err("close reported failure after cleanup")
        } else {
            Ok(())
        }
    }
    fn resize(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<(), Self::Error> {
        if self.fail == 2 {
            return Err("resize failure");
        }
        let size = self.live.get_mut(&id).ok_or("missing pane")?;
        assert!(rows > 0 && cols > 0);
        *size = (rows, cols);
        Ok(())
    }
    fn write(&mut self, id: PaneId, bytes: &[u8]) -> Result<(), Self::Error> {
        if self.fail == 3 {
            return Err("write failure");
        }
        if !self.live.contains_key(&id) {
            return Err("missing pane");
        }
        if self.output.len() == 64 {
            self.output.pop_front();
        }
        self.output.push_back((self.clock, id, bytes.to_vec()));
        Ok(())
    }
}
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = self.0;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
}
fn check(c: &Controller<Host>, area: Rect) -> Result<(), String> {
    let ids = c.session.pane_ids();
    let unique: BTreeSet<_> = ids.iter().copied().collect();
    if unique.len() != ids.len() || ids.len() > 32 {
        return Err("duplicate IDs or pane limit".into());
    }
    if unique != c.host.live.keys().copied().collect() {
        return Err(format!(
            "shell leak: domain={unique:?} live={:?}",
            c.host.live
        ));
    }
    if ids.is_empty() != c.session.focused().is_none()
        || c.session.focused().is_some_and(|id| !unique.contains(&id))
    {
        return Err("invalid focus".into());
    }
    let rects = c.session.rectangles();
    if !ids.is_empty() && rects.is_empty() {
        return Err("missing visible panes".into());
    }
    if c.session.is_zoomed()
        && rects
            != c.session
                .focused()
                .map(|id| vec![(id, area)])
                .unwrap_or_default()
    {
        return Err("zoom geometry".into());
    }
    for (i, (id, r)) in rects.iter().enumerate() {
        if !unique.contains(id) {
            return Err("visible inactive pane".into());
        }
        let (x, y, w, h) = (
            u32::from(r.x),
            u32::from(r.y),
            u32::from(r.width),
            u32::from(r.height),
        );
        if x < u32::from(area.x)
            || y < u32::from(area.y)
            || x + w > u32::from(area.x) + u32::from(area.width)
            || y + h > u32::from(area.y) + u32::from(area.height)
        {
            return Err("out of bounds geometry".into());
        }
        for (_, other) in &rects[..i] {
            if w > 0
                && h > 0
                && other.width > 0
                && other.height > 0
                && x < u32::from(other.x) + u32::from(other.width)
                && u32::from(other.x) < x + w
                && y < u32::from(other.y) + u32::from(other.height)
                && u32::from(other.y) < y + h
            {
                return Err("overlap".into());
            }
        }
    }
    if c.host.windows_order_violation() {
        return Err("output time ordering".into());
    }
    if c.notices.len() > 32 {
        return Err("unbounded notices".into());
    }
    Ok(())
}
impl Host {
    fn windows_order_violation(&self) -> bool {
        self.output
            .iter()
            .zip(self.output.iter().skip(1))
            .any(|(a, b)| a.0 > b.0)
    }
}
#[test]
fn deterministic_universe() {
    let seed: u64 = std::env::var("RMUX_TEST_SEED")
        .map(|v| v.parse().expect("RMUX_TEST_SEED must be a u64"))
        .unwrap_or(0x726d7578);
    let steps = std::env::var("RMUX_TEST_STEPS")
        .map(|v| v.parse().expect("RMUX_TEST_STEPS must be an integer"))
        .unwrap_or(5000);
    for seed in [seed, seed.wrapping_add(1), seed ^ 0xdeadbeef] {
        let mut rng = Rng(seed);
        let mut area = Rect {
            width: 120,
            height: 40,
            ..Rect::default()
        };
        let mut c = Controller::new(Session::new(area), Host::initial());
        let mut applied_area = area;
        let mut trace = VecDeque::new();
        let mut queued = VecDeque::new();
        for step in 0..steps {
            c.host.clock += rng.next() % 7;
            c.host.fail = (rng.next() % 12) as u8;
            let ids = c.session.pane_ids();
            // Recreate a universe after shutdown; pane 1 is bootstrapped by the adapter.
            if ids.is_empty() {
                c = Controller::new(Session::new(area), Host::initial());
                queued.clear();
                applied_area = area;
            }
            let event = match rng.next() % 19 {
                0..=2 => Event::Split(if rng.next() & 1 == 0 {
                    Axis::Rows
                } else {
                    Axis::Columns
                }),
                3 => Event::FocusNext,
                4 => Event::FocusPrevious,
                5 => Event::Zoom,
                6 => {
                    area = Rect {
                        width: (rng.next() % 240) as u16,
                        height: (rng.next() % 90) as u16,
                        ..Rect::default()
                    };
                    Event::Resize(area)
                }
                7 => Event::Close(c.session.focused().unwrap_or(1)),
                8 => Event::Spawned(c.host.largest.saturating_add(100)),
                9 => Event::SpawnFailed(c.host.largest.saturating_add(100)),
                10 => Event::NewWindow,
                11 => Event::WindowNext,
                12 => Event::WindowPrevious,
                13 => Event::Focus(rmux_core::Direction::Left),
                14 => Event::Focus(rmux_core::Direction::Right),
                15 => Event::Focus(rmux_core::Direction::Up),
                16 => Event::Focus(rmux_core::Direction::Down),
                17 => Event::Chrome(rmux_core::PaneChrome {
                    header: rng.next() & 1 != 0,
                    padding_x: (rng.next() % 4) as u16,
                    padding_y: (rng.next() % 4) as u16,
                }),
                _ => Event::Write(vec![(rng.next() & 255) as u8; (rng.next() % 4096) as usize]),
            };
            queued.push_back(event);
            if rng.next().is_multiple_of(3) && queued.len() < 8 {
                continue;
            }
            while let Some(event) = queued.pop_front() {
                if trace.len() == 40 {
                    trace.pop_front();
                }
                let description = match &event {
                    Event::Write(bytes) => format!("Write({} bytes)", bytes.len()),
                    other => format!("{other:?}"),
                };
                trace.push_back(format!(
                    "t={} fail={} event={description}",
                    c.host.clock, c.host.fail
                ));
                // Resize may be queued: derive the applied area, not the producer's future area.
                if let Event::Resize(r) = event {
                    applied_area = r;
                }
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.dispatch(event)));
                if let Err(error) = outcome {
                    panic!("seed={seed} step={step}: backend assertion {error:?}\ntrace={trace:#?}")
                }
                if let Err(error) = check(&c, applied_area) {
                    panic!("seed={seed} step={step}: {error}\ntrace={trace:#?}")
                }
            }
        }
    }
}

#[test]
fn async_split_interleavings_cleanup() {
    for outcome in 0..4 {
        let area = Rect {
            width: 80,
            height: 24,
            ..Rect::default()
        };
        let mut c = Controller::new(Session::new(area), Host::initial());
        let effects = c.session.apply(Event::Split(Axis::Columns));
        let Effect::Spawn { id, rows, cols } = effects[0] else {
            panic!("expected spawn")
        };
        c.host.spawn(id, rows, cols).unwrap();
        match outcome {
            0 => c.dispatch(Event::Resize(area)),
            1 => c.dispatch(Event::Close(1)),
            2 => c.dispatch(Event::SpawnFailed(id)),
            _ => {}
        }
        c.dispatch(Event::Spawned(id));
        check(&c, area).unwrap();
        c.dispatch(Event::Spawned(id));
        check(&c, area).unwrap();
    }
}

// An independent recursive geometry oracle, deliberately avoiding divide/rectangles.
fn reference(tree: &Layout, x: u16, y: u16, w: u16, h: u16, out: &mut Vec<(PaneId, Rect)>) {
    match tree {
        Layout::Pane(id) => out.push((
            *id,
            Rect {
                x,
                y,
                width: w,
                height: h,
            },
        )),
        Layout::Split {
            axis,
            first,
            second,
        } => match axis {
            Axis::Columns => {
                let usable = if w == 0 { 0 } else { w - 1 };
                reference(first, x, y, usable / 2, h, out);
                reference(
                    second,
                    x.saturating_add(usable / 2).saturating_add(1),
                    y,
                    usable - usable / 2,
                    h,
                    out,
                );
            }
            Axis::Rows => {
                let usable = if h == 0 { 0 } else { h - 1 };
                reference(first, x, y, w, usable / 2, out);
                reference(
                    second,
                    x,
                    y.saturating_add(usable / 2).saturating_add(1),
                    w,
                    usable - usable / 2,
                    out,
                );
            }
        },
    }
}
#[test]
fn seeded_layout_reference() {
    let mut rng = Rng(1234567);
    for _ in 0..1000 {
        let mut tree = Layout::Pane(1);
        for id in 2..=32 {
            assert!(tree.split(
                1 + rng.next() % (id - 1),
                id,
                if rng.next() & 1 == 0 {
                    Axis::Columns
                } else {
                    Axis::Rows
                }
            ));
        }
        let area = Rect {
            x: 3,
            y: 5,
            width: (rng.next() % 500) as u16,
            height: (rng.next() % 200) as u16,
        };
        let mut expected = Vec::new();
        reference(
            &tree,
            area.x,
            area.y,
            area.width,
            area.height,
            &mut expected,
        );
        assert_eq!(tree.rectangles(area), expected);
    }
}

#[test]
fn write_backpressure_preserves_panes_and_bounds_memory() {
    let area = Rect {
        width: 120,
        height: 40,
        ..Rect::default()
    };
    let mut c = Controller::new(Session::new(area), Host::initial());
    c.dispatch(Event::Split(Axis::Columns));
    let panes = c.session.pane_ids();
    let focus = c.session.focused();
    c.host.fail = 3;
    for _ in 0..200 {
        c.dispatch(Event::Write(vec![7; 65536]));
        assert_eq!(c.session.pane_ids(), panes);
        assert_eq!(c.session.focused(), focus);
        check(&c, area).unwrap();
    }
    assert_eq!(c.notices.len(), 32);
    c.host.fail = 0;
    c.dispatch(Event::Write(vec![7; 65537]));
    assert!(c.host.output.is_empty(), "oversized write reached host");
    c.dispatch(Event::Write(vec![7; 65536]));
    assert_eq!(c.host.output.back().unwrap().2.len(), 65536);
    // Actual writer failure arrives as its own adapter event.
    c.dispatch(Event::Close(focus.unwrap()));
    assert_eq!(c.session.pane_ids(), vec![1]);
    check(&c, area).unwrap();
}

#[test]
fn backend_faults_use_production_transitions() {
    let area = Rect {
        width: 120,
        height: 40,
        ..Rect::default()
    };
    let mut c = Controller::new(Session::new(area), Host::initial());
    c.host.fail = 1;
    c.dispatch(Event::Split(Axis::Columns));
    assert_eq!(c.session.pane_ids(), vec![1]);
    assert_eq!(c.host.live.keys().copied().collect::<Vec<_>>(), vec![1]);
    c.host.fail = 0;
    c.dispatch(Event::Split(Axis::Columns));
    assert_eq!(c.session.pane_ids(), vec![1, 3]);
    c.host.fail = 2;
    c.dispatch(Event::Resize(area));
    assert!(c.session.pane_ids().is_empty());
    assert!(c.host.live.is_empty());
    assert_eq!(c.session.focused(), None);
    let mut c = Controller::new(Session::new(area), Host::initial());
    c.host.fail = 4;
    c.dispatch(Event::Close(1));
    assert!(c.session.pane_ids().is_empty());
    assert!(c.host.live.is_empty());
    assert!(!c.notices.is_empty());
}

fn desktop() -> (Controller<Host>, Rect) {
    let area = Rect {
        width: 80,
        height: 24,
        ..Rect::default()
    };
    (Controller::new(Session::new(area), Host::initial()), area)
}

#[test]
fn fresh_session_focuses_pane_one() {
    let (session, _) = desktop();
    assert_eq!(session.session.focused(), Some(1));
    assert_eq!(session.session.pane_ids(), vec![1]);
}

#[test]
fn column_split_focuses_the_new_pane() {
    let (mut session, _) = desktop();
    session.dispatch(Event::Split(Axis::Columns));
    assert_eq!(session.session.pane_ids(), vec![1, 2]);
    assert_eq!(session.session.focused(), Some(2));
}

#[test]
fn focus_wraps_around_two_panes() {
    let (mut session, _) = desktop();
    session.dispatch(Event::Split(Axis::Rows));
    session.dispatch(Event::FocusNext);
    assert_eq!(session.session.focused(), Some(1));
    session.dispatch(Event::FocusPrevious);
    assert_eq!(session.session.focused(), Some(2));
}

#[test]
fn zoom_shows_only_the_focused_pane() {
    let (mut session, area) = desktop();
    session.dispatch(Event::Split(Axis::Columns));
    session.dispatch(Event::Zoom);
    assert_eq!(session.session.rectangles(), vec![(2, area)]);
    session.dispatch(Event::Zoom);
    assert_eq!(session.session.rectangles().len(), 2);
}

#[test]
fn closing_the_last_pane_clears_focus() {
    let (mut session, _) = desktop();
    session.dispatch(Event::Close(1));
    assert!(session.session.pane_ids().is_empty());
    assert_eq!(session.session.focused(), None);
}

#[test]
fn write_reaches_the_focused_pane() {
    let (mut session, _) = desktop();
    session.dispatch(Event::Write(b"hi".to_vec()));
    assert_eq!(session.host.output.back().unwrap().2, b"hi");
}

#[test]
fn tiny_terminal_refuses_a_split() {
    let area = Rect {
        width: 2,
        height: 2,
        ..Rect::default()
    };
    let mut session = Controller::new(Session::new(area), Host::initial());
    session.dispatch(Event::Split(Axis::Columns));
    assert_eq!(session.session.pane_ids(), vec![1]);
    assert!(!session.notices.is_empty());
}

#[test]
fn pane_limit_stops_at_the_configured_cap() {
    let area = Rect {
        width: 200,
        height: 80,
        ..Rect::default()
    };
    let mut session = Controller::new(
        Session::with_limits(area, Limits::new(2, 64).unwrap()),
        Host::initial(),
    );
    session.dispatch(Event::Split(Axis::Columns));
    session.dispatch(Event::Split(Axis::Rows));
    assert_eq!(session.session.pane_ids().len(), 2);
    assert!(!session.notices.is_empty());
}

#[test]
fn detach_keeps_the_shell() {
    let (mut session, _) = desktop();
    session.dispatch(Event::Detach);
    assert_eq!(session.session.pane_ids(), vec![1]);
    assert_eq!(session.host.live.len(), 1);
}
