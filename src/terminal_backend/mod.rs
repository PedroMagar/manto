use std::sync::mpsc::{self, Receiver, TryRecvError};

/// TerminalBackend contract (per ARCHITECTURE.md).
///
/// The backend manages process/session lifecycle:
/// - spawn: start a shell/command session
/// - write: send input bytes to a session
/// - resize: notify the session of terminal size changes
/// - kill: terminate a session
/// - poll: check for output or exit events
///
/// The trait is the public boundary; `CommandSession` currently drives the
/// platform implementations directly, and the trait will be wired into the
/// event loop drain once the emulator (Phase 2) lands.
#[allow(dead_code)]
pub trait TerminalBackend {
    type Id;

    fn spawn(&mut self, program: &str, args: &[String], cwd: Option<&str>) -> Result<Self::Id, String>;
    fn write(&mut self, id: Self::Id, data: &[u8]) -> Result<(), String>;
    fn resize(&mut self, id: Self::Id, cols: u16, rows: u16) -> Result<(), String>;
    fn kill(&mut self, id: Self::Id) -> Result<(), String>;
    fn poll(&mut self) -> Vec<TerminalEvent<Self::Id>>;
}

/// Events produced by a backend during poll().
#[allow(dead_code)]
pub enum TerminalEvent<I> {
    Output { id: I, bytes: Vec<u8> },
    Exit   { id: I, code: Option<i32> },
}

/// Internal update messages sent from the platform reader thread.
#[derive(Debug)]
pub enum TerminalUpdate {
    Line(String),
    Closed,
}

/// A persistent shell/command session backed by a platform PTY/ConPTY.
///
/// One session per terminal window. Raw input is forwarded via `write`,
/// output is drained via `poll`.
pub struct CommandSession {
    receiver:       Receiver<TerminalUpdate>,
    platform:       platform::PlatformCommand,
    closed_streams: usize,
}

pub struct CommandPoll {
    pub lines:    Vec<String>,
    pub exit_code: Option<i32>,
    pub closed:   bool,
}

impl CommandSession {
    /// Spawn a persistent interactive shell session running `program`
    /// (e.g. "/bin/sh", "powershell.exe", or $COMSPEC).
    pub fn spawn(program: &str, cwd: &str) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let platform = platform::spawn(program, cwd, tx)?;
        Ok(Self {
            receiver: rx,
            platform,
            closed_streams: 0,
        })
    }

    /// Drain pending output and check exit status.
    pub fn poll(&mut self) -> CommandPoll {
        let mut lines = Vec::new();

        loop {
            match self.receiver.try_recv() {
                Ok(TerminalUpdate::Line(line)) => lines.push(line),
                Ok(TerminalUpdate::Closed) => self.closed_streams += 1,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.closed_streams = 1;
                    break;
                }
            }
        }

        let exit_code = self.platform.try_wait();
        let closed = self.closed_streams >= 1 && exit_code.is_some();
        CommandPoll { lines, exit_code, closed }
    }

    /// Write raw bytes to the session's input.
    pub fn write(&mut self, data: &[u8]) {
        let _ = self.platform.write(data);
    }

    /// Notify the session of a terminal size change.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self.platform.resize(cols, rows);
    }

    /// Kill the session. Returns true if the process was killed.
    pub fn kill(&mut self) -> bool {
        self.platform.kill()
    }

    /// True once the session has fully exited.
    pub fn is_closed(&self) -> bool {
        self.closed_streams >= 1
    }

    /// Label of the backend in use (diagnostics/tests).
    #[cfg(test)]
    pub fn kind_label(&self) -> &'static str {
        self.platform.kind_label()
    }
}

// ── Platform selection ────────────────────────────────────────────────────────

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

#[cfg(not(windows))]
#[path = "unix.rs"]
mod platform;

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};

    fn default_shell() -> &'static str {
        #[cfg(windows)]
        {
            "cmd.exe"
        }
        #[cfg(not(windows))]
        {
            "/bin/sh"
        }
    }

    #[test]
    fn persistent_session_writes_and_reads() {
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_string_lossy().to_string();
        let mut session = CommandSession::spawn(default_shell(), &cwd).unwrap();

        // Send a command that echoes a unique marker, followed by newline.
        let marker = "manto_session_test_4711";
        #[cfg(windows)]
        let cmd = format!("echo {marker}\r\n");
        #[cfg(not(windows))]
        let cmd = format!("echo {marker}\n");
        session.write(cmd.as_bytes());

        let start = Instant::now();
        let mut saw_marker = false;
        while start.elapsed() < Duration::from_secs(5) {
            let poll = session.poll();
            for line in &poll.lines {
                if line.contains(marker) {
                    saw_marker = true;
                }
            }
            if saw_marker {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(saw_marker, "marker not echoed back by persistent session");
    }

    #[test]
    fn persistent_session_survives_multiple_commands() {
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_string_lossy().to_string();
        let mut session = CommandSession::spawn(default_shell(), &cwd).unwrap();

        #[cfg(windows)]
        let (a, b) = ("echo first_12345\r\n", "echo second_67890\r\n");
        #[cfg(not(windows))]
        let (a, b) = ("echo first_12345\n", "echo second_67890\n");

        session.write(a.as_bytes());
        let start = Instant::now();
        let mut saw_first = false;
        while start.elapsed() < Duration::from_secs(5) {
            for line in session.poll().lines {
                if line.contains("first_12345") { saw_first = true; }
            }
            if saw_first { break; }
            thread::sleep(Duration::from_millis(20));
        }

        session.write(b.as_bytes());
        let start = Instant::now();
        let mut saw_second = false;
        while start.elapsed() < Duration::from_secs(5) {
            for line in session.poll().lines {
                if line.contains("second_67890") { saw_second = true; }
            }
            if saw_second { break; }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(saw_first && saw_second, "session did not survive both commands");
    }
}
