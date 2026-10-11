use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

/// Aggregate telemetry only. Never add terminal data or pane labels here.
pub struct Metrics {
    pub panes: AtomicU64,
    pub pty_bytes: AtomicU64,
    pub frames: AtomicU64,
    pub errors: AtomicU64,
    started: Instant,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            panes: AtomicU64::new(0),
            pty_bytes: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            started: Instant::now(),
        }
    }
}

impl Metrics {
    pub fn encode(&self) -> String {
        let mut text = String::new();
        for (name, kind, help, value) in [
            (
                "rmux_panes",
                "gauge",
                "Active shell panes.",
                self.panes.load(Ordering::Relaxed),
            ),
            (
                "rmux_pty_bytes_total",
                "counter",
                "Bytes received from shell PTYs.",
                self.pty_bytes.load(Ordering::Relaxed),
            ),
            (
                "rmux_frames_total",
                "counter",
                "Terminal frames rendered.",
                self.frames.load(Ordering::Relaxed),
            ),
            (
                "rmux_errors_total",
                "counter",
                "Handled runtime errors.",
                self.errors.load(Ordering::Relaxed),
            ),
            (
                "rmux_uptime_seconds",
                "gauge",
                "Process uptime in seconds.",
                self.started.elapsed().as_secs(),
            ),
        ] {
            use std::fmt::Write;
            let _ = writeln!(
                text,
                "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}"
            );
        }
        text
    }
}

/// Bounded single-connection HTTP adapter. Disabled unless explicitly configured.
pub struct MetricsServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub address: SocketAddr,
}

impl MetricsServer {
    pub fn start(address: SocketAddr, metrics: Arc<Metrics>) -> Result<Self> {
        let listener = TcpListener::bind(address).context("bind metrics endpoint")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = thread::Builder::new().name("rmux-metrics".into()).spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut connection, _)) => {
                        let timeout = Some(Duration::from_millis(250));
                        let _ = connection.set_read_timeout(timeout);
                        let _ = connection.set_write_timeout(timeout);
                        let mut buffer = [0; 1024];
                        let mut count = 0;
                        let deadline = Instant::now() + Duration::from_millis(250);
                        let mut complete = false;
                        while count < buffer.len() && Instant::now() < deadline {
                            let remaining = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
                            let _ = connection.set_read_timeout(Some(remaining));
                            match connection.read(&mut buffer[count..]) {
                                Ok(0) | Err(_) => break,
                                Ok(received) => count += received,
                            }
                            if buffer[..count].windows(4).any(|window| window == b"\r\n\r\n") { complete = true; break; }
                        }
                        let line = buffer[..count].split(|byte| *byte == b'\n').next().unwrap_or_default();
                        let (status, body) = if !complete {
                            ("400 Bad Request", "incomplete or oversized request\n".into())
                        } else if matches!(line, b"GET /metrics HTTP/1.1\r" | b"GET /metrics HTTP/1.0\r") {
                            ("200 OK", metrics.encode())
                        } else { ("404 Not Found", "not found\n".into()) };
                        let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let _ = connection.write_all(response.as_bytes());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(25)),
                    Err(_) => { metrics.errors.fetch_add(1, Ordering::Relaxed); break; }
                }
            }
        }).context("start metrics server")?;
        Ok(Self {
            stop,
            thread: Some(thread),
            address,
        })
    }
}

impl Drop for MetricsServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exposition_is_aggregate_and_bounded() {
        let metrics = Metrics::default();
        metrics.pty_bytes.store(123, Ordering::Relaxed);
        let body = metrics.encode();
        assert!(body.contains("# TYPE rmux_pty_bytes_total counter\nrmux_pty_bytes_total 123"));
        assert_eq!(body.lines().count(), 15);
        assert!(!body.contains('{'));
        assert!(body.len() < 1024);
    }
}
