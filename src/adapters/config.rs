use std::{
    env, fs,
    io::ErrorKind,
    net::{IpAddr, SocketAddr},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use std::collections::BTreeMap;

pub const HARD_HISTORY_LIMIT: usize = 100_000;
pub const HARD_STATUS_LEN: usize = 256;
pub const HARD_COMMAND_LEN: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub server: ServerConfig,
    pub session: SessionConfig,
    pub pane: PaneConfig,
    pub input: InputConfig,
    pub keys: Keys,
    pub status: StatusConfig,
    pub style: StyleConfig,
    pub sync: SyncConfig,
    pub metrics: MetricsConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerConfig {
    pub persist: bool,
    pub socket: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    pub default_shell: String,
    pub default_command: Vec<String>,
    pub history_limit: usize,
    pub max_panes: usize,
    pub base_index: u64,
    pub write_limit_bytes: usize,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneConfig {
    pub headers: bool,
    pub padding: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputConfig {
    pub prefix: BindingKey,
    pub prefix2: Option<BindingKey>,
    pub mouse: bool,
    pub escape_time_ms: u64,
    pub repeat_time_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    pub prefix: Vec<(BindingKey, Action)>,
    pub root: Vec<(BindingKey, Action)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusConfig {
    pub top: bool,
    pub left: String,
    pub right: String,
    pub interval_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleConfig {
    pub status_fg: String,
    pub status_bg: String,
    pub pane_fg: String,
    pub pane_bg: String,
    pub active_fg: String,
    pub active_bg: String,
    pub message_fg: String,
    pub message_bg: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncConfig {
    pub mode: SyncMode,
    pub predict_echo: PredictEcho,
    pub frame_limit_bytes: usize,
    pub prediction_limit_bytes: usize,
    pub udp_address: Option<SocketAddr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    Off,
    Auto,
    LatestFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredictEcho {
    Off,
    Auto,
    On,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub address: SocketAddr,
    pub allow_remote: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingKey {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    SplitColumns,
    SplitRows,
    FocusNext,
    FocusPrevious,
    Zoom,
    ScrollUp,
    ScrollDown,
    CloseFocused,
    Quit,
    Help,
    SendPrefix,
    Cancel,
    Detach,
    RunShell(String),
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                persist: false,
                socket: "rmux.sock".into(),
            },
            session: SessionConfig {
                default_shell: String::new(),
                default_command: Vec::new(),
                history_limit: 2000,
                max_panes: 32,
                base_index: 1,
                write_limit_bytes: 65_536,
                name: "rmux".into(),
            },
            input: InputConfig {
                prefix: BindingKey {
                    code: KeyCode::Char('b'),
                    modifiers: KeyModifiers::CONTROL,
                },
                prefix2: None,
                mouse: false,
                escape_time_ms: 0,
                repeat_time_ms: 0,
            },
            pane: PaneConfig::default(),
            keys: default_keys(),
            status: StatusConfig {
                top: false,
                left: " rmux ".into(),
                right: String::new(),
                interval_ms: 1000,
            },
            style: StyleConfig {
                status_fg: "white".into(),
                status_bg: "darkblue".into(),
                pane_fg: "white".into(),
                pane_bg: "darkgrey".into(),
                active_fg: "white".into(),
                active_bg: "darkcyan".into(),
                message_fg: "black".into(),
                message_bg: "yellow".into(),
            },
            sync: SyncConfig {
                mode: SyncMode::Auto,
                predict_echo: PredictEcho::Auto,
                frame_limit_bytes: 262_144,
                prediction_limit_bytes: 256,
                udp_address: None,
            },
            metrics: MetricsConfig {
                enabled: false,
                address: "127.0.0.1:9464".parse().expect("loopback metrics address"),
                allow_remote: false,
            },
        }
    }
}

fn default_keys() -> Keys {
    let mut prefix = Vec::new();
    let bind = |map: &mut Vec<(BindingKey, Action)>, key: &str, action: Action| {
        map.push((parse_key(key).expect("default key"), action));
    };
    bind(&mut prefix, "%", Action::SplitColumns);
    bind(&mut prefix, "\"", Action::SplitRows);
    bind(&mut prefix, "o", Action::FocusNext);
    bind(&mut prefix, "Tab", Action::FocusNext);
    bind(&mut prefix, "p", Action::FocusPrevious);
    bind(&mut prefix, "z", Action::Zoom);
    bind(&mut prefix, "PageUp", Action::ScrollUp);
    bind(&mut prefix, "PageDown", Action::ScrollDown);
    bind(&mut prefix, "x", Action::CloseFocused);
    bind(&mut prefix, "q", Action::Quit);
    bind(&mut prefix, "?", Action::Help);
    bind(&mut prefix, "C-b", Action::SendPrefix);
    bind(&mut prefix, "Esc", Action::Cancel);
    bind(&mut prefix, "d", Action::Detach);
    Keys {
        prefix,
        root: Vec::new(),
    }
}

impl Config {
    pub fn pane_layout(&self) -> Result<rmux_core::PaneLayout> {
        rmux_core::PaneLayout::new(self.pane.headers, self.pane.padding)
            .map_err(|error| anyhow::anyhow!(error))
    }
    pub fn shell(&self) -> String {
        if !self.session.default_shell.is_empty() {
            return self.session.default_shell.clone();
        }
        env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }

    pub fn is_prefix(&self, key: &KeyEvent) -> bool {
        self.matches(&self.input.prefix, key)
            || self
                .input
                .prefix2
                .as_ref()
                .is_some_and(|prefix| self.matches(prefix, key))
    }

    pub fn binding(&self, prefixed: bool, key: &KeyEvent) -> Option<&Action> {
        let table = if prefixed {
            &self.keys.prefix
        } else {
            &self.keys.root
        };
        table
            .iter()
            .find(|(binding, _)| self.matches(binding, key))
            .map(|(_, action)| action)
    }

    fn matches(&self, binding: &BindingKey, key: &KeyEvent) -> bool {
        binding.code == key.code && binding.modifiers == key.modifiers
    }

    pub fn prefix_label(&self) -> String {
        display_key(&self.input.prefix)
    }

    pub fn help_line(&self) -> String {
        let bindings = self
            .keys
            .prefix
            .iter()
            .map(|(key, action)| {
                let label = match action {
                    Action::SplitColumns => "columns",
                    Action::SplitRows => "rows",
                    Action::FocusNext => "next pane",
                    Action::FocusPrevious => "previous pane",
                    Action::Zoom => "zoom",
                    Action::ScrollUp => "scroll up",
                    Action::ScrollDown => "scroll down",
                    Action::CloseFocused => "close",
                    Action::Quit => "quit",
                    Action::Help => "help",
                    Action::SendPrefix => "send prefix",
                    Action::Cancel => "cancel",
                    Action::Detach => "detach",
                    Action::RunShell(_) => "run shell",
                };
                format!("{} {label}", display_key(key))
            })
            .collect::<Vec<_>>()
            .join(" | ");
        format!(" {} : {bindings} ", self.prefix_label())
    }

    pub fn status_left(&self) -> String {
        expand_status(&self.status.left, &self.session.name)
            .unwrap_or_else(|_| self.status.left.clone())
    }

    pub fn summary(&self) -> String {
        format!(
            "persist = {}\nprefix = {}\nmax_panes = {}\nhistory_limit = {}\nwrite_limit_bytes = {}\nmetrics = {}\n",
            self.server.persist,
            display_key(&self.input.prefix),
            self.session.max_panes,
            self.session.history_limit,
            self.session.write_limit_bytes,
            if self.metrics.enabled {
                self.metrics.address.to_string()
            } else {
                "off".into()
            },
        )
    }
}

pub fn user_config_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("rmux/rmux.toml"))
}

pub fn load(explicit: Option<&Path>) -> Result<Config> {
    if let Some(path) = explicit {
        return load_path(path, 0, &mut Vec::new());
    }
    match user_config_path() {
        Some(path) if path.exists() => load_path(&path, 0, &mut Vec::new()),
        _ => Ok(Config::default()),
    }
}

fn load_path(path: &Path, depth: usize, seen: &mut Vec<PathBuf>) -> Result<Config> {
    if depth > 8 {
        bail!("config include depth exceeded");
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("open {}", path.display()))?;
    if seen.iter().any(|item| item == &canonical) {
        bail!("config include cycle at {}", canonical.display());
    }
    trusted(&canonical)?;
    seen.push(canonical.clone());
    let text =
        fs::read_to_string(&canonical).with_context(|| format!("read {}", canonical.display()))?;
    let file: FileConfig =
        toml::from_str(&text).with_context(|| format!("parse {}", canonical.display()))?;
    let mut config = Config::default();
    if let Some(includes) = file.include.clone() {
        for include in includes {
            let next = PathBuf::from(&include);
            let next = if next.is_absolute() {
                next
            } else {
                canonical
                    .parent()
                    .context("config path has no parent")?
                    .join(next)
            };
            config = load_path(&next, depth + 1, seen)?;
        }
    }
    file.overlay(&mut config)?;
    validate(&config)?;
    Ok(config)
}

fn trusted(path: &Path) -> Result<()> {
    let meta = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if meta.mode() & 0o022 != 0 {
        bail!("{} must not be group or world writable", path.display());
    }
    if meta.uid() != current_uid() {
        bail!("{} must be owned by the current user", path.display());
    }
    Ok(())
}

fn current_uid() -> u32 {
    nix::unistd::Uid::current().as_raw()
}

fn validate(config: &Config) -> Result<()> {
    config.pane_layout()?;
    rmux_core::Limits::new(config.session.max_panes, config.session.write_limit_bytes)
        .map_err(|error| anyhow::anyhow!(error))?;
    if config.session.history_limit > HARD_HISTORY_LIMIT {
        bail!("history_limit exceeds {HARD_HISTORY_LIMIT}");
    }
    if config.session.base_index == 0 {
        bail!("base_index starts at 1");
    }
    if config.sync.frame_limit_bytes == 0
        || config.sync.frame_limit_bytes > rmux_core::HARD_FRAME_LIMIT
    {
        bail!("frame_limit_bytes exceeds {}", rmux_core::HARD_FRAME_LIMIT);
    }
    if config.sync.prediction_limit_bytes > rmux_core::HARD_PREDICTION_LIMIT {
        bail!(
            "prediction_limit_bytes exceeds {}",
            rmux_core::HARD_PREDICTION_LIMIT
        );
    }
    if config.input.escape_time_ms > 5_000 || config.input.repeat_time_ms > 5_000 {
        bail!("input timing exceeds 5000ms");
    }
    if config.status.interval_ms > 60_000 {
        bail!("status interval exceeds 60000ms");
    }
    for text in [
        &config.status.left,
        &config.status.right,
        &config.session.name,
    ] {
        if text.len() > HARD_STATUS_LEN {
            bail!("status text exceeds {HARD_STATUS_LEN} bytes");
        }
    }
    expand_status(&config.status.left, &config.session.name)?;
    expand_status(&config.status.right, &config.session.name)?;
    if config
        .sync
        .udp_address
        .is_some_and(|address| address.ip().is_unspecified())
    {
        bail!("sync.udp_address must not be an unspecified address");
    }
    check_metrics(&config.metrics)?;
    for color in [
        &config.style.status_fg,
        &config.style.status_bg,
        &config.style.pane_fg,
        &config.style.pane_bg,
        &config.style.active_fg,
        &config.style.active_bg,
        &config.style.message_fg,
        &config.style.message_bg,
    ] {
        named_color(color).with_context(|| format!("unknown color {color}"))?;
    }
    Ok(())
}

pub fn check_metrics(metrics: &MetricsConfig) -> Result<()> {
    if !metrics.allow_remote && !is_loopback(metrics.address.ip()) {
        bail!("metrics address must be loopback unless allow_remote = true");
    }
    Ok(())
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
    }
}

pub fn named_color(name: &str) -> Result<crossterm::style::Color> {
    use crossterm::style::Color;
    Ok(match name {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "darkgrey" | "darkgray" => Color::DarkGrey,
        "darkred" => Color::DarkRed,
        "darkgreen" => Color::DarkGreen,
        "darkyellow" => Color::DarkYellow,
        "darkblue" => Color::DarkBlue,
        "darkmagenta" => Color::DarkMagenta,
        "darkcyan" => Color::DarkCyan,
        "default" => Color::Reset,
        other => bail!("unknown color {other}"),
    })
}

fn expand_status(text: &str, name: &str) -> Result<String> {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '#' {
            match chars.next() {
                Some('#') => out.push('#'),
                Some('S') => out.push_str(name),
                Some(other) => bail!("unknown status token #{other}"),
                None => bail!("truncated status token"),
            }
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

pub fn parse_key(raw: &str) -> Result<BindingKey> {
    let mut modifiers = KeyModifiers::empty();
    let mut rest = raw;
    loop {
        if let Some(next) = rest.strip_prefix("C-") {
            modifiers |= KeyModifiers::CONTROL;
            rest = next;
        } else if let Some(next) = rest.strip_prefix("M-") {
            modifiers |= KeyModifiers::ALT;
            rest = next;
        } else if let Some(next) = rest.strip_prefix("S-") {
            modifiers |= KeyModifiers::SHIFT;
            rest = next;
        } else {
            break;
        }
    }
    let code = match rest {
        "Tab" => KeyCode::Tab,
        "Esc" => KeyCode::Esc,
        "Enter" => KeyCode::Enter,
        "Space" => KeyCode::Char(' '),
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Backspace" => KeyCode::Backspace,
        other if other.chars().count() == 1 => KeyCode::Char(other.chars().next().unwrap()),
        other => bail!("unknown key {other}"),
    };
    if modifiers.is_empty() && matches!(code, KeyCode::Char(ch) if ch.is_ascii_uppercase()) {
        bail!("use S- for shifted letters");
    }
    Ok(BindingKey { code, modifiers })
}

fn display_key(key: &BindingKey) -> String {
    let mut text = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        text.push_str("Ctrl-");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        text.push_str("Alt-");
    }
    match key.code {
        KeyCode::Char(ch) => text.push(ch),
        KeyCode::Tab => text.push_str("Tab"),
        KeyCode::Esc => text.push_str("Esc"),
        other => text.push_str(&format!("{other:?}")),
    }
    text
}

fn parse_action(raw: &str) -> Result<Action> {
    if let Some(command) = raw.strip_prefix("run-shell:") {
        if command.is_empty() || command.len() > HARD_COMMAND_LEN {
            bail!("run-shell command must be 1..={HARD_COMMAND_LEN} bytes");
        }
        return Ok(Action::RunShell(command.to_string()));
    }
    Ok(match raw {
        "split-columns" => Action::SplitColumns,
        "split-rows" => Action::SplitRows,
        "focus-next" => Action::FocusNext,
        "focus-previous" => Action::FocusPrevious,
        "zoom" => Action::Zoom,
        "scroll-up" => Action::ScrollUp,
        "scroll-down" => Action::ScrollDown,
        "close-focused" => Action::CloseFocused,
        "quit" => Action::Quit,
        "help" => Action::Help,
        "send-prefix" => Action::SendPrefix,
        "cancel" => Action::Cancel,
        "detach" => Action::Detach,
        "copy-mode" => bail!("copy-mode is not available yet"),
        other => bail!("unknown action {other}"),
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    include: Option<Vec<String>>,
    server: Option<ServerFile>,
    session: Option<SessionFile>,
    pane: Option<PaneFile>,
    input: Option<InputFile>,
    keys: Option<KeysFile>,
    status: Option<StatusFile>,
    style: Option<StyleFile>,
    sync: Option<SyncFile>,
    metrics: Option<MetricsFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerFile {
    persist: Option<bool>,
    socket: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionFile {
    default_shell: Option<String>,
    default_command: Option<Vec<String>>,
    history_limit: Option<usize>,
    max_panes: Option<usize>,
    base_index: Option<u64>,
    write_limit_bytes: Option<usize>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PaneFile {
    headers: Option<bool>,
    padding: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputFile {
    prefix: Option<String>,
    prefix2: Option<String>,
    mouse: Option<bool>,
    escape_time_ms: Option<u64>,
    repeat_time_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysFile {
    prefix: Option<BTreeMap<String, String>>,
    root: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusFile {
    position: Option<String>,
    left: Option<String>,
    right: Option<String>,
    interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleFile {
    status_fg: Option<String>,
    status_bg: Option<String>,
    pane_fg: Option<String>,
    pane_bg: Option<String>,
    active_fg: Option<String>,
    active_bg: Option<String>,
    message_fg: Option<String>,
    message_bg: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncFile {
    mode: Option<String>,
    predict_echo: Option<String>,
    frame_limit_bytes: Option<usize>,
    prediction_limit_bytes: Option<usize>,
    udp_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricsFile {
    enabled: Option<bool>,
    address: Option<String>,
    allow_remote: Option<bool>,
}

impl FileConfig {
    fn overlay(self, config: &mut Config) -> Result<()> {
        if let Some(pane) = self.pane {
            if let Some(headers) = pane.headers {
                config.pane.headers = headers;
            }
            if let Some(padding) = pane.padding {
                config.pane.padding = padding;
            }
        }
        if let Some(server) = self.server {
            if let Some(persist) = server.persist {
                config.server.persist = persist;
            }
            if let Some(socket) = server.socket {
                if socket.is_empty() || socket.contains('/') || socket.contains('\0') {
                    bail!("server.socket must be a file name");
                }
                config.server.socket = socket;
            }
        }
        if let Some(session) = self.session {
            if let Some(shell) = session.default_shell {
                config.session.default_shell = shell;
            }
            if let Some(command) = session.default_command {
                if command.len() > 32 || command.iter().any(|arg| arg.len() > HARD_COMMAND_LEN) {
                    bail!("default_command is too large");
                }
                config.session.default_command = command;
            }
            if let Some(history) = session.history_limit {
                config.session.history_limit = history;
            }
            if let Some(max_panes) = session.max_panes {
                config.session.max_panes = max_panes;
            }
            if let Some(base_index) = session.base_index {
                config.session.base_index = base_index;
            }
            if let Some(limit) = session.write_limit_bytes {
                config.session.write_limit_bytes = limit;
            }
            if let Some(name) = session.name {
                config.session.name = name;
            }
        }
        if let Some(input) = self.input {
            if let Some(prefix) = input.prefix {
                config.input.prefix = parse_key(&prefix)?;
            }
            if let Some(prefix2) = input.prefix2 {
                config.input.prefix2 = if prefix2.is_empty() {
                    None
                } else {
                    Some(parse_key(&prefix2)?)
                };
            }
            if let Some(mouse) = input.mouse {
                config.input.mouse = mouse;
            }
            if let Some(escape) = input.escape_time_ms {
                config.input.escape_time_ms = escape;
            }
            if let Some(repeat) = input.repeat_time_ms {
                config.input.repeat_time_ms = repeat;
            }
        }
        if let Some(keys) = self.keys {
            if let Some(prefix) = keys.prefix {
                config.keys.prefix = parse_table(prefix)?;
            }
            if let Some(root) = keys.root {
                config.keys.root = parse_table(root)?;
            }
        }
        if let Some(status) = self.status {
            if let Some(position) = status.position {
                config.status.top = match position.as_str() {
                    "top" => true,
                    "bottom" => false,
                    other => bail!("unknown status position {other}"),
                };
            }
            if let Some(left) = status.left {
                config.status.left = left;
            }
            if let Some(right) = status.right {
                config.status.right = right;
            }
            if let Some(interval) = status.interval_ms {
                config.status.interval_ms = interval;
            }
        }
        if let Some(style) = self.style {
            assign(&mut config.style.status_fg, style.status_fg);
            assign(&mut config.style.status_bg, style.status_bg);
            assign(&mut config.style.pane_fg, style.pane_fg);
            assign(&mut config.style.pane_bg, style.pane_bg);
            assign(&mut config.style.active_fg, style.active_fg);
            assign(&mut config.style.active_bg, style.active_bg);
            assign(&mut config.style.message_fg, style.message_fg);
            assign(&mut config.style.message_bg, style.message_bg);
        }
        if let Some(sync) = self.sync {
            if let Some(mode) = sync.mode {
                config.sync.mode = match mode.as_str() {
                    "off" => SyncMode::Off,
                    "auto" => SyncMode::Auto,
                    "latest-frame" => SyncMode::LatestFrame,
                    other => bail!("unknown sync mode {other}"),
                };
            }
            if let Some(predict) = sync.predict_echo {
                config.sync.predict_echo = match predict.as_str() {
                    "off" => PredictEcho::Off,
                    "auto" => PredictEcho::Auto,
                    "on" => PredictEcho::On,
                    other => bail!("unknown predict_echo {other}"),
                };
            }
            if let Some(limit) = sync.frame_limit_bytes {
                config.sync.frame_limit_bytes = limit;
            }
            if let Some(limit) = sync.prediction_limit_bytes {
                config.sync.prediction_limit_bytes = limit;
            }
            if let Some(address) = sync.udp_address {
                config.sync.udp_address = if address.is_empty() {
                    None
                } else {
                    Some(address.parse().context("parse sync.udp_address")?)
                };
            }
        }
        if let Some(metrics) = self.metrics {
            if let Some(enabled) = metrics.enabled {
                config.metrics.enabled = enabled;
            }
            if let Some(address) = metrics.address {
                config.metrics.address = address.parse().context("parse metrics.address")?;
            }
            if let Some(allow_remote) = metrics.allow_remote {
                config.metrics.allow_remote = allow_remote;
            }
        }
        Ok(())
    }
}

fn assign(slot: &mut String, value: Option<String>) {
    if let Some(value) = value {
        *slot = value;
    }
}

fn parse_table(raw: BTreeMap<String, String>) -> Result<Vec<(BindingKey, Action)>> {
    let mut table = Vec::new();
    for (key, action) in raw {
        let key = parse_key(&key)?;
        let action = parse_action(&action)?;
        if table.iter().any(|(existing, _)| existing == &key) {
            bail!("duplicate key binding");
        }
        table.push((key, action));
    }
    Ok(table)
}

pub fn runtime_dir() -> Result<PathBuf> {
    let base = env::var_os("RMUX_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
        .unwrap_or_else(|| env::temp_dir().join(format!("rmux-{}", current_uid())));
    fs::create_dir_all(&base)?;
    let mut perms = fs::metadata(&base)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(&base, perms)?;
    Ok(base)
}

pub fn socket_path(config: &Config) -> Result<PathBuf> {
    Ok(runtime_dir()?.join(&config.server.socket))
}

pub fn print_default_config() -> String {
    r#"# rmux defaults. Missing keys keep these values.
[server]
persist = false
socket = "rmux.sock"

[session]
default_shell = ""
default_command = []
history_limit = 2000
max_panes = 32
base_index = 1
write_limit_bytes = 65536
name = "rmux"

[pane]
headers = false
padding = 0

[input]
prefix = "C-b"
prefix2 = ""
mouse = false
escape_time_ms = 0
repeat_time_ms = 0

[status]
position = "bottom"
left = " rmux "
right = ""
interval_ms = 1000

[style]
status_fg = "white"
status_bg = "darkblue"
pane_fg = "white"
pane_bg = "darkgrey"
active_fg = "white"
active_bg = "darkcyan"
message_fg = "black"
message_bg = "yellow"

[sync]
mode = "auto"
predict_echo = "auto"
frame_limit_bytes = 262144
prediction_limit_bytes = 256
udp_address = ""

[metrics]
enabled = false
address = "127.0.0.1:9464"
allow_remote = false
"#
    .to_string()
}

pub fn missing_is_default(path: &Path) -> bool {
    matches!(fs::metadata(path), Err(error) if error.kind() == ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_owned(path: &Path, text: &str, mode: u32) {
        fs::write(path, text).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(mode);
        fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    fn missing_file_equals_defaults() {
        let path = env::temp_dir().join(format!("rmux-missing-{}-config.toml", std::process::id()));
        let _ = fs::remove_file(&path);
        assert!(missing_is_default(&path));
        assert_eq!(Config::default().input.prefix.code, KeyCode::Char('b'));
        assert!(
            Config::default()
                .binding(
                    true,
                    &KeyEvent::new(KeyCode::Char('%'), KeyModifiers::empty())
                )
                .is_some()
        );
    }

    #[test]
    fn pane_layout_defaults_round_trip_and_validate() {
        let mut config = Config::default();
        let file: FileConfig = toml::from_str(&print_default_config()).unwrap();
        file.overlay(&mut config).unwrap();
        validate(&config).unwrap();
        assert_eq!(config, Config::default());
        assert!(!config.pane.headers);
        assert_eq!(config.pane.padding, 0);
        let file: FileConfig = toml::from_str("[pane]\nheaders = true\npadding = 16\n").unwrap();
        file.overlay(&mut config).unwrap();
        validate(&config).unwrap();
        assert!(config.pane_layout().unwrap().headers);
        config.pane.padding = 17;
        assert!(validate(&config).is_err());
        for text in [
            "[pane]\npadding = -1",
            "[pane]\npadding = 65536",
            "[pane]\nunknown = true",
        ] {
            assert!(toml::from_str::<FileConfig>(text).is_err());
        }
    }

    #[test]
    fn custom_prefix_binding_dispatches_without_a_terminal() {
        let dir = env::temp_dir().join(format!("rmux-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rmux.toml");
        write_owned(
            &path,
            "[input]\nprefix = \"C-a\"\n[keys.prefix]\n\"%\" = \"split-columns\"\n",
            0o600,
        );
        let config = load_path(&path, 0, &mut Vec::new()).unwrap();
        let key = KeyEvent::new(KeyCode::Char('%'), KeyModifiers::empty());
        assert_eq!(config.binding(true, &key), Some(&Action::SplitColumns));
        assert!(config.is_prefix(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)));
        assert_eq!(config.help_line(), " Ctrl-a : % columns ");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_key_and_over_cap_are_rejected() {
        let dir = env::temp_dir().join(format!("rmux-config-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rmux.toml");
        write_owned(&path, "not_a_section = true\n", 0o600);
        assert!(load_path(&path, 0, &mut Vec::new()).is_err());
        write_owned(&path, "[session]\nmax_panes = 100\n", 0o600);
        assert!(load_path(&path, 0, &mut Vec::new()).is_err());
        write_owned(&path, "[metrics]\naddress = \"0.0.0.0:9464\"\n", 0o600);
        assert!(load_path(&path, 0, &mut Vec::new()).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn world_writable_file_is_rejected() {
        let dir = env::temp_dir().join(format!("rmux-config-mode-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rmux.toml");
        write_owned(&path, "", 0o666);
        assert!(load_path(&path, 0, &mut Vec::new()).is_err());
        write_owned(&path, "", 0o600);
        assert!(load_path(&path, 0, &mut Vec::new()).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}
