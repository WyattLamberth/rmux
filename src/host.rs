use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rmux::adapters::{
    config::{self, Config},
    pty::Pane,
    render,
};
use rmux_core::{PaneHost, PaneId};

pub struct ShellHost {
    pub shell: String,
    pub command: Vec<String>,
    pub history_limit: usize,
    pub panes: BTreeMap<PaneId, Pane>,
    pub spawn_dir: Option<std::path::PathBuf>,
}

impl PaneHost for ShellHost {
    type Error = anyhow::Error;
    fn spawn(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<()> {
        anyhow::ensure!(!self.panes.contains_key(&id), "duplicate pane identifier");
        let cwd = self.spawn_dir.take().unwrap_or(std::env::current_dir()?);
        let pane = if self.command.is_empty() {
            Pane::spawn_in(&self.shell, &[], rows, cols, self.history_limit, &cwd)?
        } else {
            Pane::spawn_in(
                &self.command[0],
                &self.command[1..],
                rows,
                cols,
                self.history_limit,
                &cwd,
            )?
        };
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

pub fn chrome(config: &Config) -> render::Chrome {
    let color = |name: &str| config::named_color(name).unwrap_or(crossterm::style::Color::Reset);
    render::Chrome {
        message_fg: color(&config.style.message_fg),
        message_bg: color(&config.style.message_bg),
        status_fg: color(&config.style.status_fg),
        status_bg: color(&config.style.status_bg),
        pane_fg: color(&config.style.pane_fg),
        pane_bg: color(&config.style.pane_bg),
        active_fg: color(&config.style.active_fg),
        active_bg: color(&config.style.active_bg),
        top: config.status.top,
        headers: config.pane.headers,
        border_fg: color(&config.style.pane_fg),
        active_border_fg: color(&config.style.active_bg),
        right: config.status_right(),
        left_length: config.status.left_length,
        right_length: config.status.right_length,
        left: config.status_left(),
    }
}

pub fn area(width: u16, height: u16, top: bool) -> rmux_core::Rect {
    if top {
        rmux_core::Rect {
            x: 0,
            y: u16::from(height > 0),
            width,
            height: height.saturating_sub(1),
        }
    } else {
        rmux_core::Rect {
            x: 0,
            y: 0,
            width,
            height: height.saturating_sub(1),
        }
    }
}

pub fn split(app: &mut rmux_core::Controller<ShellHost>, event: rmux_core::Event, config: &Config) {
    app.host.spawn_dir = if config.pane.inherit_cwd {
        app.session
            .focused()
            .and_then(|id| app.host.panes.get(&id))
            .and_then(Pane::current_dir)
    } else {
        None
    };
    app.dispatch(event);
    // A rejected split must not leave a directory queued for a later spawn.
    app.host.spawn_dir = None;
}

pub fn configure(
    app: &mut rmux_core::Controller<ShellHost>,
    config: &Config,
    dimensions: (u16, u16),
) {
    app.dispatch(rmux_core::Event::Chrome(config.pane.chrome()));
    app.dispatch(rmux_core::Event::Resize(area(
        dimensions.0,
        dimensions.1,
        config.status.top,
    )));
    app.host.shell = config.shell();
    app.host.command = config.session.default_command.clone();
    app.host.history_limit = config.session.history_limit;
}
pub fn reload(current: &Config, path: Option<&std::path::Path>) -> Result<Config> {
    let mut next = config::load(path)?;
    next.shell_override = current.shell_override.clone();
    next.persist_override = current.persist_override;
    if let Some(shell) = &next.shell_override {
        next.session.default_shell = shell.clone();
    }
    if next.persist_override {
        next.server.persist = true;
    }
    anyhow::ensure!(
        next.server == current.server
            && next.metrics == current.metrics
            && next.sync == current.sync
            && next.session.max_panes == current.session.max_panes
            && next.session.write_limit_bytes == current.session.write_limit_bytes,
        "server, metrics, sync and session limits require a restart"
    );
    Ok(next)
}
pub fn mouse(app: &mut rmux_core::Controller<ShellHost>, column: u16, row: u16, scroll: i8) {
    let id = app
        .session
        .rectangles()
        .into_iter()
        .find(|(_, r)| {
            column >= r.x
                && column < r.x.saturating_add(r.width)
                && row >= r.y
                && row < r.y.saturating_add(r.height)
        })
        .map(|(id, _)| id);
    if let Some(id) = id {
        app.dispatch(rmux_core::Event::FocusPane(id));
        if scroll != 0
            && let Some(pane) = app.host.panes.get_mut(&id)
        {
            pane.scroll(isize::from(scroll) * 3);
        }
    }
}
pub fn mouse_scroll(kind: crossterm::event::MouseEventKind) -> Option<i8> {
    use crossterm::event::{MouseButton, MouseEventKind};
    match kind {
        MouseEventKind::Down(MouseButton::Left) => Some(0),
        MouseEventKind::ScrollUp => Some(1),
        MouseEventKind::ScrollDown => Some(-1),
        _ => None,
    }
}
