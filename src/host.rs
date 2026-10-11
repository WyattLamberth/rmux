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
}

impl PaneHost for ShellHost {
    type Error = anyhow::Error;
    fn spawn(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<()> {
        anyhow::ensure!(!self.panes.contains_key(&id), "duplicate pane identifier");
        let pane = if self.command.is_empty() {
            Pane::spawn_with(&self.shell, &[], rows, cols, self.history_limit)?
        } else {
            Pane::spawn_with(
                &self.command[0],
                &self.command[1..],
                rows,
                cols,
                self.history_limit,
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
        status_fg: color(&config.style.status_fg),
        status_bg: color(&config.style.status_bg),
        pane_fg: color(&config.style.pane_fg),
        pane_bg: color(&config.style.pane_bg),
        active_fg: color(&config.style.active_fg),
        active_bg: color(&config.style.active_bg),
        top: config.status.top,
    }
}

pub fn area(width: u16, height: u16, top: bool) -> rmux_core::Rect {
    if top {
        rmux_core::Rect {
            x: 0,
            y: 1,
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
