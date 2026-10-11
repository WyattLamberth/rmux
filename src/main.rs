use std::{
    collections::BTreeMap,
    env,
    io::{self, IsTerminal, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self, Event as TerminalEvent, KeyCode, KeyEventKind, KeyModifiers},
    terminal,
};
use rmux::adapters::{
    input::{encode_key, encode_paste},
    metrics::{Metrics, MetricsServer},
    pty::Pane,
    render,
    terminal::Terminal,
};
use rmux_core::{Axis, Controller, Event, PaneHost, PaneId, Rect, Session};

struct ShellHost {
    shell: String,
    panes: BTreeMap<PaneId, Pane>,
}

impl PaneHost for ShellHost {
    type Error = anyhow::Error;
    fn spawn(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<()> {
        anyhow::ensure!(!self.panes.contains_key(&id), "duplicate pane identifier");
        let pane = Pane::spawn(&self.shell, rows, cols)?;
        self.panes.insert(id, pane);
        Ok(())
    }
    fn close(&mut self, id: PaneId) -> Result<()> {
        self.panes.remove(&id);
        Ok(())
    }
    fn resize(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<()> {
        self.panes
            .get_mut(&id)
            .context("pane is no longer available")?
            .resize(rows, cols)
    }
    fn write(&mut self, id: PaneId, bytes: &[u8]) -> Result<()> {
        self.panes
            .get_mut(&id)
            .context("pane is no longer available")?
            .write(bytes)
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("rmux: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                println!(
                    "rmux {} — AI-built Rust terminal multiplexer\n\nUsage: rmux [--shell PATH]\n\nPrefix: Ctrl-b, then:\n  %       split into columns\n  \"       split into rows\n  o / Tab next pane\n  p       previous pane\n  z       toggle zoom\n  PageUp  scroll up\n  PageDown scroll down\n  x       close focused shell (press y to confirm)\n  q       quit all shells (press y to confirm)\n  ?       show help\n  Ctrl-b  send literal Ctrl-b\n  Esc     cancel prefix or confirmation\n\nMetrics: opt in with RMUX_METRICS_ADDR=127.0.0.1:9464\nPersistent and remote sessions are planned; this build runs attached shells.",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            "--version" | "-V" => {
                println!("rmux {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--shell" => shell = args.next().context("--shell requires an executable path")?,
            _ => bail!("unknown argument {argument:?}; use --help"),
        }
    }
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "run rmux in an interactive terminal"
    );
    let metrics = Arc::new(Metrics::default());
    let _metrics_server = match env::var("RMUX_METRICS_ADDR") {
        Ok(address) => Some(MetricsServer::start(
            address
                .parse()
                .context("RMUX_METRICS_ADDR must be an IP:port")?,
            Arc::clone(&metrics),
        )?),
        Err(env::VarError::NotPresent) => None,
        Err(error) => return Err(error).context("read metrics address"),
    };
    let stopped = Arc::new(AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(signal, Arc::clone(&stopped))
            .context("register shutdown signals")?;
    }
    let mut dimensions = terminal::size().context("get terminal size")?;
    let session = Session::new(area(dimensions));
    let mut host = ShellHost {
        shell,
        panes: BTreeMap::new(),
    };
    host.spawn(
        1,
        dimensions.1.saturating_sub(2).max(1),
        dimensions.0.max(1),
    )?;
    let mut app = Controller::new(session, host);
    let _terminal = Terminal::enter()?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        rmux::adapters::terminal::restore();
        previous_hook(information);
    }));
    let mut prefix = false;
    let mut confirmation = None;
    let mut status =
        String::from(" rmux | Ctrl-b ? help | % columns | \" rows | o focus | z zoom ");
    let mut dirty = true;
    let mut last_frame = Instant::now() - Duration::from_secs(1);
    let mut stdout = io::stdout().lock();
    while !app.host.panes.is_empty() && !stopped.load(Ordering::Relaxed) {
        let mut ended = Vec::new();
        for (&id, pane) in &mut app.host.panes {
            match pane.drain() {
                Ok(bytes) => {
                    dirty |= bytes > 0;
                    metrics.pty_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
                }
                Err(_) => {
                    metrics.errors.fetch_add(1, Ordering::Relaxed);
                    ended.push(id);
                }
            }
            if pane.exited()? {
                ended.push(id);
            }
        }
        for id in ended {
            app.dispatch(Event::Close(id));
            dirty = true;
        }
        metrics
            .panes
            .store(app.host.panes.len() as u64, Ordering::Relaxed);
        if !app.notices.is_empty() {
            metrics
                .errors
                .fetch_add(app.notices.len() as u64, Ordering::Relaxed);
            if let Some(notice) = app.notices.last() {
                status = format!(" {notice} | Ctrl-b ? help ");
            }
            app.notices.clear();
            dirty = true;
        }
        if app.host.panes.is_empty() {
            break;
        }
        if dirty && last_frame.elapsed() >= Duration::from_millis(16) {
            let frame = render::draw(&app.session, &app.host.panes, dimensions, &status)?;
            stdout.write_all(&frame).context("render terminal")?;
            stdout.flush()?;
            metrics.frames.fetch_add(1, Ordering::Relaxed);
            last_frame = Instant::now();
            dirty = false;
        }
        if !event::poll(Duration::from_millis(8))? {
            continue;
        }
        match event::read()? {
            TerminalEvent::Resize(width, height) => {
                dimensions = (width, height);
                app.dispatch(Event::Resize(area(dimensions)));
                dirty = true;
            }
            TerminalEvent::Paste(text) => {
                if confirmation.is_some() || prefix {
                    continue;
                }
                if let Some(pane) = app.session.focused().and_then(|id| app.host.panes.get(&id)) {
                    if text.len() > 65_524 {
                        status = " Paste exceeds 64 KiB limit; use smaller chunks ".into();
                    } else {
                        app.dispatch(Event::Write(encode_paste(
                            &text,
                            pane.screen().bracketed_paste(),
                        )));
                    }
                    dirty = true;
                }
            }
            TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
                dirty = true;
                if let Some(close_all) = confirmation.take() {
                    if key.code == KeyCode::Char('y') {
                        let ids = if close_all {
                            app.session.pane_ids()
                        } else {
                            app.session.focused().into_iter().collect()
                        };
                        for id in ids {
                            app.dispatch(Event::Close(id));
                        }
                    }
                    status = " rmux | Ctrl-b ? help ".into();
                    continue;
                }
                if prefix {
                    prefix = false;
                    status = " rmux | Ctrl-b ? help ".into();
                    match key.code {
                        KeyCode::Char('%') => app.dispatch(Event::Split(Axis::Columns)),
                        KeyCode::Char('"') => app.dispatch(Event::Split(Axis::Rows)),
                        KeyCode::Char('o') | KeyCode::Tab => app.dispatch(Event::FocusNext),
                        KeyCode::Char('p') | KeyCode::BackTab => app.dispatch(Event::FocusPrevious),
                        KeyCode::Char('z') => app.dispatch(Event::Zoom),
                        KeyCode::Char('x') => { confirmation = Some(false); status = " Close focused shell? y confirms; any other key cancels ".into(); }
                        KeyCode::Char('q') => { confirmation = Some(true); status = " Quit and terminate all shells? y confirms; any other key cancels ".into(); }
                        KeyCode::Char('?') => status = " Ctrl-b: % columns | \" rows | o next | p previous | z zoom | PgUp/PgDn scroll | x close | q quit ".into(),
                        KeyCode::PageUp | KeyCode::PageDown => {
                            if let Some(pane) = app.session.focused().and_then(|id| app.host.panes.get_mut(&id)) { pane.scroll(if key.code == KeyCode::PageUp { 10 } else { -10 }); }
                        }
                        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => app.dispatch(Event::Write(vec![2])),
                        _ => {}
                    }
                } else if key.code == KeyCode::Char('b')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    prefix = true;
                    status = " Prefix: % columns | \" rows | o next | p previous | z zoom | x close | q quit | Esc cancel ".into();
                } else if let Some(pane) =
                    app.session.focused().and_then(|id| app.host.panes.get(&id))
                {
                    let bytes = encode_key(key, pane.screen().application_cursor());
                    if !bytes.is_empty() {
                        app.dispatch(Event::Write(bytes));
                    }
                }
            }
            _ => {}
        }
    }
    // Release stdout before guard cleanup re-locks it. ShellHost drops and reaps shells.
    drop(stdout);
    Ok(())
}

fn area((width, height): (u16, u16)) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width,
        height: height.saturating_sub(1),
    }
}
