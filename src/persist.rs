use std::{
    fs, io,
    io::{IsTerminal, Read, Write},
    net::UdpSocket,
    os::unix::{fs::PermissionsExt, net::UnixStream, process::CommandExt},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self, Event as TerminalEvent, KeyCode, KeyEventKind},
    terminal,
};
use rmux::adapters::{
    config::{self, Action, Config, PredictEcho, SyncMode},
    input::encode_key,
    metrics::{Metrics, MetricsServer},
    protocol::{self, ClientMsg, ServerMsg},
    render,
    terminal::Terminal,
};
use rmux_core::{
    Axis, Controller, EchoPrediction, Event, Limits, PaneHost, ScreenSync, Session, SessionKey,
};

use crate::host::{ShellHost, area, chrome};

pub fn run_persistent(
    config: &Config,
    config_path: Option<&str>,
    shell_override: Option<&str>,
) -> Result<()> {
    if config::socket_path(config)
        .ok()
        .is_some_and(|path| path.exists())
    {
        return attach_local(config);
    }
    spawn_server(config_path, shell_override)?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if config::socket_path(config)
            .ok()
            .is_some_and(|path| path.exists())
        {
            return attach_local(config);
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!("session server did not open a socket")
}

pub fn attach_local(config: &Config) -> Result<()> {
    let stream = connect(config)?;
    let writer = stream.try_clone().context("clone session socket")?;
    client_loop(config, stream, writer)
}

pub fn attach_ssh(config: &Config, target: &str) -> Result<()> {
    let mut child = Command::new("ssh")
        .args([
            "-T",
            "-o",
            "BatchMode=yes",
            target,
            "--",
            "rmux",
            "serve",
            "--stdio",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("start ssh")?;
    let stdin = child.stdin.take().context("ssh stdin")?;
    let stdout = child.stdout.take().context("ssh stdout")?;
    let result = client_loop(config, stdout, stdin);
    let _ = child.wait();
    result
}

pub fn serve(config: &Config, stdio: bool) -> Result<()> {
    let limits = Limits::new(config.session.max_panes, config.session.write_limit_bytes)
        .map_err(|error| anyhow::anyhow!(error))?;
    let mut dimensions = (80u16, 24u16);
    let mut app = Controller::new(
        Session::with_limits(area(dimensions.0, dimensions.1, config.status.top), limits),
        ShellHost {
            shell: config.shell(),
            command: config.session.default_command.clone(),
            history_limit: config.session.history_limit,
            panes: Default::default(),
        },
    );
    app.host.spawn(1, 22, 80)?;
    let metrics = Arc::new(Metrics::default());
    let _metrics = start_metrics(config, &metrics)?;
    let stopped = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        let _ = signal_hook::flag::register(signal, Arc::clone(&stopped));
    }
    let key = session_key()?;
    let udp = bind_udp(config)?;
    let udp_port = udp
        .as_ref()
        .and_then(|socket| socket.local_addr().ok())
        .map(|addr| addr.port())
        .unwrap_or(0);
    let listener = if stdio { None } else { Some(listen(config)?) };
    let mut client = if stdio {
        Some(ServerLink::spawn(io::stdin(), io::stdout()))
    } else {
        None
    };
    let mut sync = ScreenSync::default();
    let mut epoch = 1u64;
    let mut status = config.status_left();
    let mut dirty = true;
    let mut last_frame = Instant::now() - Duration::from_secs(1);
    let mut output_cursor = 0usize;
    let mut last_udp = None;
    while !app.host.panes.is_empty() && !stopped.load(Ordering::Relaxed) {
        dirty |= drain(&mut app, &metrics, &mut output_cursor);
        if client.is_none()
            && let Some(listener) = &listener
            && let Ok((stream, _)) = listener.accept()
            && same_user(&stream)
            && let Ok(link) = ServerLink::from_unix(stream)
        {
            client = Some(link);
            dirty = true;
        }
        if let Some(socket) = &udp {
            let mut packet = [0; 65_536];
            if let Ok((len, source)) = socket.recv_from(&mut packet)
                && let Some(msg) = decode_udp(&key, &packet[..len], config.sync.frame_limit_bytes)
            {
                last_udp = Some(source);
                if apply_message(&mut app, &msg, &mut dimensions, &mut status, config) {
                    break;
                }
                dirty = true;
            }
        }
        let mut drop_client = false;
        let mut quit = false;
        if let Some(link) = client.as_mut() {
            match link.incoming.try_recv() {
                Ok(Ok(msg)) => {
                    if matches!(msg, ClientMsg::Hello { .. }) {
                        let _ = protocol::write_server(
                            &mut link.writer,
                            &ServerMsg::Hello { key, udp_port },
                        );
                    }
                    if matches!(msg, ClientMsg::Detach) {
                        app.dispatch(Event::Detach);
                        let _ = protocol::write_server(&mut link.writer, &ServerMsg::Bye);
                        drop_client = true;
                    } else if apply_message(&mut app, &msg, &mut dimensions, &mut status, config) {
                        let _ = protocol::write_server(&mut link.writer, &ServerMsg::Bye);
                        quit = true;
                    }
                    dirty = true;
                }
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => drop_client = true,
                Err(TryRecvError::Empty) => {}
            }
        }
        if drop_client {
            client = None;
        }
        if quit {
            break;
        }
        if dirty && last_frame.elapsed() >= Duration::from_millis(16) {
            let frame = render::draw(
                &app.session,
                &app.host.panes,
                dimensions,
                &status,
                &chrome(config),
            )
            .context("render frame")?;
            epoch = epoch.saturating_add(1);
            if sync
                .publish(epoch, &frame, config.sync.frame_limit_bytes)
                .is_ok()
            {
                if let Some(link) = client.as_mut() {
                    let _ = protocol::write_server(
                        &mut link.writer,
                        &ServerMsg::Frame {
                            epoch,
                            bytes: frame.clone(),
                        },
                    );
                }
                if let (Some(socket), Some(source)) = (&udp, last_udp) {
                    let _ = send_udp(socket, source, &key, epoch, &frame);
                }
            }
            metrics.frames.fetch_add(1, Ordering::Relaxed);
            last_frame = Instant::now();
            dirty = false;
        }
        if stdio && client.is_none() {
            break;
        }
        thread::sleep(Duration::from_millis(4));
    }
    if let Ok(path) = config::socket_path(config) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn apply_message(
    app: &mut Controller<ShellHost>,
    msg: &ClientMsg,
    dimensions: &mut (u16, u16),
    status: &mut String,
    config: &Config,
) -> bool {
    match msg {
        ClientMsg::Hello { rows, cols } | ClientMsg::Resize { rows, cols } => {
            *dimensions = (*cols, *rows);
            app.dispatch(Event::Resize(area(*cols, *rows, config.status.top)));
        }
        ClientMsg::Write(bytes) => app.dispatch(Event::Write(bytes.clone())),
        ClientMsg::Action(code) => return apply_code(app, *code, status, config),
        ClientMsg::Detach => app.dispatch(Event::Detach),
        ClientMsg::Quit => return true,
    }
    false
}

fn apply_code(
    app: &mut Controller<ShellHost>,
    code: u8,
    status: &mut String,
    config: &Config,
) -> bool {
    match code {
        1 => app.dispatch(Event::Split(Axis::Columns)),
        2 => app.dispatch(Event::Split(Axis::Rows)),
        3 => app.dispatch(Event::FocusNext),
        4 => app.dispatch(Event::FocusPrevious),
        5 => app.dispatch(Event::Zoom),
        6 | 7 => {
            if let Some(pane) = app
                .session
                .focused()
                .and_then(|id| app.host.panes.get_mut(&id))
            {
                pane.scroll(if code == 6 { 10 } else { -10 });
            }
        }
        8 => {
            if let Some(id) = app.session.focused() {
                app.dispatch(Event::Close(id));
            }
        }
        9 => return true,
        10 => *status = config.help_line(),
        13 => app.dispatch(Event::Detach),
        _ => {}
    }
    false
}

fn client_loop(
    config: &Config,
    reader: impl Read + Send + 'static,
    mut writer: impl Write + Send + 'static,
) -> Result<()> {
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "run rmux in an interactive terminal"
    );
    let dimensions = terminal::size().context("get terminal size")?;
    protocol::write_client(
        &mut writer,
        &ClientMsg::Hello {
            rows: dimensions.1,
            cols: dimensions.0,
        },
    )?;
    writer.flush()?;
    let (tx, incoming) = mpsc::sync_channel(8);
    thread::spawn(move || {
        let mut reader = reader;
        loop {
            match protocol::read_server(&mut reader) {
                Ok(msg) => {
                    if tx.send(Ok(msg)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                    break;
                }
            }
        }
    });
    let hello = recv_server(&incoming)?;
    if !matches!(hello, ServerMsg::Hello { .. }) {
        bail!("expected session hello");
    }
    let _terminal = Terminal::enter()?;
    let mut echo = EchoPrediction::new(config.sync.prediction_limit_bytes);
    let predict = !matches!(config.sync.predict_echo, PredictEcho::Off)
        && !matches!(config.sync.mode, SyncMode::Off);
    let mut prefix = false;
    let mut confirmation = None;
    let mut stdout = io::stdout().lock();
    loop {
        match incoming.try_recv() {
            Ok(Ok(ServerMsg::Frame { bytes, .. })) => {
                if predict {
                    echo.rollback();
                }
                stdout.write_all(&bytes)?;
                stdout.flush()?;
            }
            Ok(Ok(ServerMsg::Bye)) => break,
            Ok(Ok(ServerMsg::Hello { .. })) => {}
            Ok(Err(error)) => return Err(error).context("session closed"),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => break,
        }
        if !event::poll(Duration::from_millis(8))? {
            continue;
        }
        match event::read()? {
            TerminalEvent::Resize(cols, rows) => {
                protocol::write_client(&mut writer, &ClientMsg::Resize { rows, cols })?;
            }
            TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
                if let Some(close_all) = confirmation.take() {
                    if key.code == KeyCode::Char('y') {
                        protocol::write_client(
                            &mut writer,
                            &ClientMsg::Action(if close_all { 9 } else { 8 }),
                        )?;
                        if close_all {
                            break;
                        }
                    }
                    continue;
                }
                if prefix {
                    prefix = false;
                    if let Some(action) = config.binding(true, &key).cloned()
                        && dispatch_action(action, &mut writer, &mut confirmation)?
                    {
                        break;
                    }
                    continue;
                }
                if config.is_prefix(&key) {
                    prefix = true;
                    continue;
                }
                let bytes = encode_key(key, false);
                if predict && !bytes.is_empty() {
                    echo.push(&bytes);
                }
                if !bytes.is_empty() {
                    protocol::write_client(&mut writer, &ClientMsg::Write(bytes))?;
                }
            }
            TerminalEvent::Paste(text)
                if !prefix && confirmation.is_none() && text.len() <= 65_524 =>
            {
                protocol::write_client(&mut writer, &ClientMsg::Write(text.into_bytes()))?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn dispatch_action(
    action: Action,
    writer: &mut impl Write,
    confirmation: &mut Option<bool>,
) -> Result<bool> {
    let detach = matches!(action, Action::Detach);
    match action {
        Action::Detach => protocol::write_client(writer, &ClientMsg::Detach)?,
        Action::Quit => *confirmation = Some(true),
        Action::CloseFocused => *confirmation = Some(false),
        Action::Help => protocol::write_client(writer, &ClientMsg::Action(10))?,
        Action::SendPrefix => protocol::write_client(writer, &ClientMsg::Write(vec![2]))?,
        Action::Cancel => {}
        Action::RunShell(command) => {
            let _ = Command::new("sh").arg("-c").arg(command).spawn();
        }
        other => {
            if let Some(code) = protocol::action_code(&other) {
                protocol::write_client(writer, &ClientMsg::Action(code))?;
            }
        }
    }
    Ok(detach)
}

fn recv_server(incoming: &Receiver<io::Result<ServerMsg>>) -> Result<ServerMsg> {
    let started = Instant::now();
    loop {
        match incoming.try_recv() {
            Ok(Ok(msg)) => return Ok(msg),
            Ok(Err(error)) => return Err(error).context("session hello"),
            Err(TryRecvError::Empty) if started.elapsed() < Duration::from_secs(2) => {
                thread::sleep(Duration::from_millis(8));
            }
            Err(_) => bail!("session server closed before hello"),
        }
    }
}

struct ServerLink {
    incoming: Receiver<io::Result<ClientMsg>>,
    writer: Box<dyn Write + Send>,
}

impl ServerLink {
    fn spawn(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static) -> Self {
        let (tx, incoming) = mpsc::sync_channel(16);
        thread::spawn(move || {
            let mut reader = reader;
            loop {
                match protocol::read_client(&mut reader) {
                    Ok(msg) => {
                        if tx.send(Ok(msg)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        break;
                    }
                }
            }
        });
        Self {
            incoming,
            writer: Box::new(writer),
        }
    }

    fn from_unix(stream: UnixStream) -> io::Result<Self> {
        let writer = stream.try_clone()?;
        Ok(Self::spawn(stream, writer))
    }
}

fn connect(config: &Config) -> Result<UnixStream> {
    let path = config::socket_path(config)?;
    UnixStream::connect(&path).with_context(|| format!("connect {}", path.display()))
}

fn listen(config: &Config) -> Result<std::os::unix::net::UnixListener> {
    let path = config::socket_path(config)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let mut perms = fs::metadata(parent)?.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(parent, perms)?;
    }
    let _ = fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path)
        .with_context(|| format!("bind {}", path.display()))?;
    let mut perms = fs::metadata(&path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&path, perms)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn same_user(stream: &UnixStream) -> bool {
    let Ok(creds) =
        nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
    else {
        return false;
    };
    creds.uid() == nix::unistd::Uid::current().as_raw()
}

fn spawn_server(config_path: Option<&str>, shell_override: Option<&str>) -> Result<()> {
    let exe = std::env::current_exe().context("find rmux")?;
    let mut command = Command::new(exe);
    command.arg("serve");
    if let Some(path) = config_path {
        command.args(["--config", path]);
    }
    if let Some(shell) = shell_override {
        command.args(["--shell", shell]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    command.spawn().context("spawn session server")?;
    Ok(())
}

fn session_key() -> Result<[u8; 32]> {
    let mut key = [0; 32];
    fs::File::open("/dev/urandom")
        .context("open random")?
        .read_exact(&mut key)?;
    Ok(key)
}

fn bind_udp(config: &Config) -> Result<Option<UdpSocket>> {
    let Some(address) = config.sync.udp_address else {
        return Ok(None);
    };
    let socket = UdpSocket::bind(address).context("bind sync udp")?;
    socket.set_nonblocking(true)?;
    Ok(Some(socket))
}

fn decode_udp(key: &[u8; 32], packet: &[u8], limit: usize) -> Option<ClientMsg> {
    if packet.len() < 32 {
        return None;
    }
    let mut presented = [0; 32];
    presented.copy_from_slice(&packet[..32]);
    let body = rmux_core::open_packet(
        &SessionKey(*key),
        &SessionKey(presented),
        &packet[32..],
        limit,
    )
    .ok()?;
    decode_client_body(body).ok()
}

fn decode_client_body(body: &[u8]) -> io::Result<ClientMsg> {
    protocol::read_client(
        &mut &{
            let mut encoded = Vec::new();
            encoded.extend((body.len() as u32).to_be_bytes());
            encoded.extend(body);
            encoded
        }[..],
    )
}

fn send_udp(
    socket: &UdpSocket,
    source: std::net::SocketAddr,
    key: &[u8; 32],
    epoch: u64,
    frame: &[u8],
) -> Result<()> {
    let mut encoded = Vec::new();
    protocol::write_server(
        &mut encoded,
        &ServerMsg::Frame {
            epoch,
            bytes: frame.to_vec(),
        },
    )?;
    let mut packet = Vec::new();
    packet.extend(key);
    packet.extend(&encoded[4..]);
    socket.send_to(&packet, source)?;
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
            Err(_) => ended.push(id),
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

fn start_metrics(config: &Config, metrics: &Arc<Metrics>) -> Result<Option<MetricsServer>> {
    if !config.metrics.enabled {
        return Ok(None);
    }
    config::check_metrics(&config.metrics)?;
    Ok(Some(MetricsServer::start(
        config.metrics.address,
        Arc::clone(metrics),
    )?))
}
