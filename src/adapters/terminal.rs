use std::io::{self, Write};

use anyhow::{Context, Result};
use crossterm::{
    cursor::{Hide, Show},
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    style::{Attribute, ResetColor, SetAttribute},
    terminal::{
        EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};

pub struct Terminal;

impl Terminal {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("enable terminal raw mode")?;
        let guard = Self;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide
        )
        .context("prepare terminal")?;
        Ok(guard)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

pub fn restore() {
    let _ = execute!(
        io::stdout(),
        EndSynchronizedUpdate,
        SetAttribute(Attribute::Reset),
        ResetColor,
        Show,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    let _ = io::stdout().flush();
    let _ = disable_raw_mode();
}
