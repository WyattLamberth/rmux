#![forbid(unsafe_code)]
use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self, Event as TerminalEvent},
    terminal,
};
use portable_pty::CommandBuilder;
use rmux::{
    input::{Action, Input},
    pty::Pty,
    session::{Effect, Event, PaneId, Session},
    ui::{self, TerminalGuard},
};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, IsTerminal},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "rmux [-- COMMAND [ARG...]]\n\nRun $SHELL (or /bin/sh) in an attached multiplexer.\nCtrl-b then: % split columns, \" split rows, o focus, z zoom,\n[ scrollback (arrows/PgUp/PgDown, q/Esc returns), x close, q quit.\nCtrl-b Ctrl-b sends Ctrl-b. Paste limit: 16 KiB. Linux/Unix only."
        );
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg != "--") || (args.len() == 1) {
        bail!("usage: rmux [-- COMMAND [ARG...]]");
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("rmux requires a terminal on stdin and stdout");
    }
    let mut command = if let Some(program) = args.get(1) {
        CommandBuilder::new(program)
    } else {
        CommandBuilder::new(std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into()))
    };
    if args.len() > 2 {
        command.args(&args[2..]);
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut signals = Vec::new();
    for signal in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGINT,
    ] {
        signals.push(signal_hook::flag::register(signal, Arc::clone(&shutdown))?);
    }
    let result = run(command, shutdown);
    for id in signals {
        signal_hook::low_level::unregister(id);
    }
    result
}

fn effects(
    session: &mut Session,
    ptys: &mut BTreeMap<PaneId, Pty>,
    initial: Vec<Effect>,
    command: &CommandBuilder,
    notice: &mut String,
) -> Result<()> {
    let mut pending: VecDeque<_> = initial.into();
    while let Some(effect) = pending.pop_front() {
        let failure = match effect {
            Effect::Spawn(id, rect) => match Pty::spawn(command.clone(), rect) {
                Ok(pty) => {
                    ptys.insert(id, pty);
                    pending.extend(session.apply(Event::Spawned(id)));
                    None
                }
                Err(_) => {
                    *notice = format!("pane {}: shell spawn failed", id.0);
                    Some(id)
                }
            },
            Effect::Resize(id, rect) => ptys
                .get(&id)
                .and_then(|pty| pty.resize(rect).err().map(|_| id)),
            Effect::Write(id, bytes) => {
                if let Some(pty) = ptys.get_mut(&id) {
                    if !pty.enqueue(&bytes) {
                        *notice = "input rejected: pane queue full".into();
                    }
                }
                None
            }
            Effect::Stop(id) => {
                if let Some(mut pty) = ptys.remove(&id) {
                    pty.stop().context("clean up pane")?;
                }
                None
            }
        };
        if let Some(id) = failure {
            pending.extend(session.apply(Event::Failed(id)));
        }
    }
    Ok(())
}

fn run(command: CommandBuilder, shutdown: Arc<AtomicBool>) -> Result<()> {
    let guard = TerminalGuard::enter().context("enter terminal mode")?;
    let (mut cols, mut rows) = terminal::size()?;
    let (mut session, initial) = Session::new(cols, rows);
    // PTYs drop before the terminal guard on all return/error paths.
    let mut ptys = BTreeMap::new();
    let mut notice = "Ctrl-b then ? for commands".to_string();
    effects(&mut session, &mut ptys, initial, &command, &mut notice)?;
    let mut input = Input::default();
    let mut dirty = true;
    let mut quit = false;
    while !quit && !shutdown.load(Ordering::Relaxed) {
        for id in ptys.keys().copied().collect::<Vec<_>>() {
            let tick = match ptys.get_mut(&id) {
                Some(pty) => pty.tick(),
                None => continue,
            };
            let updates = match tick {
                Ok(tick) => {
                    let mut updates: Vec<_> = tick
                        .output
                        .into_iter()
                        .map(|bytes| Event::Output(id, bytes))
                        .collect();
                    if let Some(code) = tick.exit {
                        updates.push(Event::Exited(id, code));
                    }
                    updates
                }
                Err(_) => {
                    notice = format!("pane {}: PTY I/O failed", id.0);
                    vec![Event::Failed(id)]
                }
            };
            for update in updates {
                let work = session.apply(update);
                effects(&mut session, &mut ptys, work, &command, &mut notice)?;
                dirty = true;
            }
        }
        if dirty {
            ui::render(&mut io::stdout().lock(), &session, cols, rows, &notice)?;
            dirty = false;
        }
        if session.finished() {
            break;
        }
        for index in 0..32 {
            if !event::poll(if index == 0 {
                Duration::from_millis(16)
            } else {
                Duration::ZERO
            })? {
                break;
            }
            let screen = session
                .focus()
                .and_then(|id| session.panes().get(&id))
                .map(|p| p.terminal.screen());
            let action = match event::read()? {
                TerminalEvent::Key(key) => input.key(key, screen),
                TerminalEvent::Paste(text) => {
                    Some(input.paste(&text, screen.is_some_and(vt100::Screen::bracketed_paste)))
                }
                TerminalEvent::Resize(c, r) => {
                    cols = c;
                    rows = r;
                    Some(Action::Session(Event::Resize { cols, rows }))
                }
                _ => None,
            };
            match action {
                Some(Action::Quit) => {
                    quit = true;
                    break;
                }
                Some(Action::Notice(text)) => notice = text.into(),
                Some(Action::Session(update)) => {
                    let work = session.apply(update);
                    effects(&mut session, &mut ptys, work, &command, &mut notice)?;
                }
                None => continue,
            }
            dirty = true;
        }
    }
    for pty in ptys.values_mut() {
        pty.stop()?;
    }
    drop(ptys);
    drop(guard);
    Ok(())
}
