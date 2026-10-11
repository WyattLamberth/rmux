mod host;
mod persist;

use std::{
    env,
    io::{self, IsTerminal, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self, Event as TerminalEvent, KeyCode, KeyEventKind},
    terminal,
};
use rmux::adapters::{
    config::{self, Action, Config, MetricsConfig},
    input::{encode_key, encode_paste},
    metrics::{Metrics, MetricsServer},
    render,
    terminal::Terminal,
};
use rmux_core::{Axis, Controller, Event, Limits, PaneHost, Session};

use host::{ShellHost, area, chrome};

fn main() {
    if let Err(error) = run() {
        eprintln!("rmux: {error:#}");
        std::process::exit(1);
    }
}

struct Args {
    config: Option<PathBuf>,
    shell: Option<String>,
    persist: bool,
    print_config: bool,
    print_default: bool,
    serve: bool,
    stdio: bool,
    attach: Option<String>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        config: None,
        shell: None,
        persist: false,
        print_config: false,
        print_default: false,
        serve: false,
        stdio: false,
        attach: None,
    };
    let mut rest = env::args().skip(1);
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("rmux {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--print-default-config" => args.print_default = true,
            "--print-config" => args.print_config = true,
            "--persist" => args.persist = true,
            "--stdio" => args.stdio = true,
            "--config" => {
                args.config = Some(PathBuf::from(
                    rest.next().context("--config requires a path")?,
                ));
            }
            "--shell" => args.shell = Some(rest.next().context("--shell requires a path")?),
            "serve" => args.serve = true,
            "attach" => args.attach = Some(rest.next().unwrap_or_default()),
            other => bail!("unknown argument {other:?}; use --help"),
        }
    }
    Ok(args)
}

fn print_help() {
    println!(
        "rmux {} — terminal multiplexer\n\nUsage:\n  rmux [--shell PATH] [--config PATH] [--persist]\n  rmux attach [user@host]\n  rmux serve [--stdio]\n  rmux --print-config\n  rmux --print-default-config\n\nPrefix: Ctrl-b, then % \" o p z x q d ?\nConfig: $XDG_CONFIG_HOME/rmux/rmux.toml",
        env!("CARGO_PKG_VERSION")
    );
}

fn run() -> Result<()> {
    let args = parse_args()?;
    if args.print_default {
        print!("{}", config::print_default_config());
        return Ok(());
    }
    let mut config = config::load(args.config.as_deref())?;
    if let Some(shell) = &args.shell {
        config.session.default_shell = shell.clone();
    }
    if args.persist {
        config.server.persist = true;
    }
    if args.print_config {
        print!("{}", config.summary());
        return Ok(());
    }
    if args.serve {
        return persist::serve(&config, args.stdio);
    }
    if let Some(target) = args.attach {
        if target.is_empty() {
            return persist::attach_local(&config);
        }
        return persist::attach_ssh(&config, &target);
    }
    if config.server.persist {
        let config_path = args
            .config
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        return persist::run_persistent(&config, config_path.as_deref(), args.shell.as_deref());
    }
    attached(config)
}

fn attached(config: Config) -> Result<()> {
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "run rmux in an interactive terminal"
    );
    let metrics = Arc::new(Metrics::default());
    let _metrics_server = metrics_server(&config, &metrics)?;
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
    let limits = Limits::new(config.session.max_panes, config.session.write_limit_bytes)
        .map_err(|error| anyhow::anyhow!(error))?;
    let mut app = Controller::new(
        Session::with_limits(area(dimensions.0, dimensions.1, config.status.top), limits),
        ShellHost {
            shell: config.shell(),
            command: config.session.default_command.clone(),
            history_limit: config.session.history_limit,
            panes: Default::default(),
        },
    );
    app.host.spawn(
        1,
        dimensions.1.saturating_sub(2).max(1),
        dimensions.0.max(1),
    )?;
    let _terminal = Terminal::enter()?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        rmux::adapters::terminal::restore();
        previous_hook(information);
    }));
    let mut prefix = false;
    let mut confirmation = None;
    let mut status = config.status_left();
    let mut dirty = true;
    let mut last_frame = Instant::now() - Duration::from_secs(1);
    let mut output_cursor = 0usize;
    let chrome = chrome(&config);
    let mut stdout = io::stdout().lock();
    while !app.host.panes.is_empty() && !stopped.load(Ordering::Relaxed) {
        dirty |= drain(&mut app, &metrics, &mut output_cursor);
        if !app.notices.is_empty() {
            metrics
                .errors
                .fetch_add(app.notices.len() as u64, Ordering::Relaxed);
            if let Some(notice) = app.notices.last() {
                status = format!(" {notice} ");
            }
            app.notices.clear();
            dirty = true;
        }
        if app.host.panes.is_empty() {
            break;
        }
        if dirty && last_frame.elapsed() >= Duration::from_millis(16) {
            let frame = render::draw(&app.session, &app.host.panes, dimensions, &status, &chrome)?;
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
                app.dispatch(Event::Resize(area(width, height, config.status.top)));
                dirty = true;
            }
            TerminalEvent::Paste(text) => {
                if confirmation.is_some() || prefix {
                    continue;
                }
                if let Some(pane) = app.session.focused().and_then(|id| app.host.panes.get(&id)) {
                    if text.len() > config.session.write_limit_bytes.saturating_sub(12) {
                        status = " Paste exceeds the write limit ".into();
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
                    status = config.status_left();
                    continue;
                }
                if prefix {
                    prefix = false;
                    status = config.status_left();
                    if let Some(action) = config.binding(true, &key).cloned() {
                        match action {
                            Action::SplitColumns => app.dispatch(Event::Split(Axis::Columns)),
                            Action::SplitRows => app.dispatch(Event::Split(Axis::Rows)),
                            Action::FocusNext => app.dispatch(Event::FocusNext),
                            Action::FocusPrevious => app.dispatch(Event::FocusPrevious),
                            Action::Zoom => app.dispatch(Event::Zoom),
                            Action::ScrollUp | Action::ScrollDown => {
                                if let Some(pane) = app
                                    .session
                                    .focused()
                                    .and_then(|id| app.host.panes.get_mut(&id))
                                {
                                    pane.scroll(if matches!(action, Action::ScrollUp) {
                                        10
                                    } else {
                                        -10
                                    });
                                }
                            }
                            Action::CloseFocused => {
                                confirmation = Some(false);
                                status = " Close focused shell? y confirms ".into();
                            }
                            Action::Quit => {
                                confirmation = Some(true);
                                status = " Quit and terminate all shells? y confirms ".into();
                            }
                            Action::Help => status = config.help_line(),
                            Action::SendPrefix => app.dispatch(Event::Write(vec![2])),
                            Action::Cancel => {}
                            Action::Detach => {
                                status = " Detach needs --persist or server.persist ".into();
                            }
                            Action::RunShell(command) => {
                                let _ = std::process::Command::new("sh")
                                    .arg("-c")
                                    .arg(command)
                                    .spawn();
                            }
                        }
                    }
                } else if config.is_prefix(&key) {
                    prefix = true;
                    status = format!(" Prefix {} ", config.prefix_label());
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
    drop(stdout);
    Ok(())
}

fn drain(app: &mut Controller<ShellHost>, metrics: &Metrics, output_cursor: &mut usize) -> bool {
    let mut dirty = false;
    let mut ended = Vec::new();
    let ids: Vec<_> = app.host.panes.keys().copied().collect();
    if ids.is_empty() {
        return false;
    }
    let inspected = ids.len().min(16);
    for offset in 0..inspected {
        let id = ids[(*output_cursor + offset) % ids.len()];
        let Some(pane) = app.host.panes.get_mut(&id) else {
            continue;
        };
        match pane.drain_budget(4) {
            Ok(bytes) => {
                dirty |= bytes > 0;
                metrics.pty_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
            }
            Err(_) => {
                metrics.errors.fetch_add(1, Ordering::Relaxed);
                ended.push(id);
            }
        }
        if pane.exited().unwrap_or(true) {
            ended.push(id);
        }
    }
    *output_cursor = (*output_cursor + inspected) % ids.len().max(1);
    for id in ended {
        app.dispatch(Event::Close(id));
        dirty = true;
    }
    metrics
        .panes
        .store(app.host.panes.len() as u64, Ordering::Relaxed);
    dirty
}

fn metrics_server(config: &Config, metrics: &Arc<Metrics>) -> Result<Option<MetricsServer>> {
    if let Ok(value) = env::var("RMUX_METRICS_ADDR") {
        if value.is_empty() {
            return Ok(None);
        }
        let address: SocketAddr = value
            .parse()
            .context("RMUX_METRICS_ADDR must be an IP:port")?;
        config::check_metrics(&MetricsConfig {
            enabled: true,
            address,
            allow_remote: config.metrics.allow_remote,
        })?;
        return Ok(Some(MetricsServer::start(address, Arc::clone(metrics))?));
    }
    if !config.metrics.enabled {
        return Ok(None);
    }
    config::check_metrics(&config.metrics)?;
    Ok(Some(MetricsServer::start(
        config.metrics.address,
        Arc::clone(metrics),
    )?))
}
