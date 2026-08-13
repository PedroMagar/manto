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
/// The trait is the public boundary between host and emulator/UI. `CommandSession`
/// is the single-session implementation (`Id = ()`, one session per terminal
/// window); `spawn` re-hosts the session on the same instance. The raw event
/// loop drain goes through the contract: `TerminalState::tick` polls via
/// `TerminalBackend::poll`, and `write`/`resize`/`kill` are the contract's own
/// spellings of the platform primitives.
///
/// The `spawn` method is exercised by the trait-contract tests: production
/// hosts sessions through `CommandSession::spawn`/`spawn_app` because the
/// shell and app flavors cannot share one command-line form (e.g. the Windows
/// persistent-shell pipe fallback vs the app bootstrap).
#[cfg_attr(not(test), allow(dead_code))]
pub trait TerminalBackend {
    type Id;

    fn spawn(
        &mut self,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<Self::Id, String>;
    fn write(&mut self, id: Self::Id, data: &[u8]) -> Result<(), String>;
    fn resize(&mut self, id: Self::Id, cols: u16, rows: u16) -> Result<(), String>;
    fn kill(&mut self, id: Self::Id) -> Result<(), String>;
    fn poll(&mut self) -> Vec<TerminalEvent<Self::Id>>;
}

/// Events produced by a backend during poll(). In the single-session
/// implementation `I = ()`; `id` is made concrete (unit) by the drain to keep
/// the multi-session contract visible.
pub enum TerminalEvent<I> {
    Output { id: I, bytes: Vec<u8> },
    Exit { id: I, code: Option<i32> },
}

/// Update messages from the platform reader thread: raw output chunks exactly
/// as read (CR/LF, UTF-8 tails and ANSI sequences preserved) plus EOF.
#[derive(Debug)]
pub enum TerminalUpdate {
    Output(Vec<u8>),
    Closed,
}

/// A persistent shell/command session backed by a platform PTY/ConPTY.
///
/// One session per terminal window. Raw input is forwarded via `write`,
/// output is drained via `poll`.
pub struct CommandSession {
    receiver: Receiver<TerminalUpdate>,
    platform: platform::PlatformCommand,
    closed_streams: usize,
}

pub struct CommandPoll {
    /// Raw output chunks drained since the last poll, in order.
    pub outputs: Vec<Vec<u8>>,
    /// Exit code of the session, if it has exited.
    pub exit_code: Option<i32>,
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

    /// Spawn a session running `program` directly (interactive apps, editors,
    /// REPLs). On Windows the bare program name is resolved through PATH so a
    /// real executable is started even when `CreateProcessW` cannot handle
    /// app-execution aliases; the piped fallback bootstraps the program
    /// through `cmd` instead of silently swapping it for a shell.
    pub fn spawn_app(program: &str, cwd: &str) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let platform = platform::spawn_app(program, cwd, tx)?;
        Ok(Self {
            receiver: rx,
            platform,
            closed_streams: 0,
        })
    }

    /// Spawn a session running `program` with explicit arguments. This is the
    /// trait-compliant spawn: the platform renders the command line from the
    /// program and its args exactly as a typed command would be parsed.
    pub fn spawn_with_args(
        program: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<Self, String> {
        let cwd = cwd.unwrap_or(".");
        let (tx, rx) = mpsc::channel();
        let platform = platform::spawn_argv(program, args, cwd, tx)?;
        Ok(Self {
            receiver: rx,
            platform,
            closed_streams: 0,
        })
    }

    /// Drain pending output and check exit status.
    pub fn poll(&mut self) -> CommandPoll {
        let mut outputs = Vec::new();

        loop {
            match self.receiver.try_recv() {
                Ok(TerminalUpdate::Output(bytes)) => outputs.push(bytes),
                Ok(TerminalUpdate::Closed) => self.closed_streams += 1,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.closed_streams = 1;
                    break;
                }
            }
        }

        let exit_code = self.platform.try_wait();
        CommandPoll { outputs, exit_code }
    }

    /// Write raw bytes to the session's input.
    pub fn write(&mut self, data: &[u8]) {
        let _ = TerminalBackend::write(self, (), data);
    }

    /// Notify the session of a terminal size change.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = TerminalBackend::resize(self, (), cols, rows);
    }

    /// Kill the session. Returns true when the kill request was dispatched.
    pub fn kill(&mut self) -> bool {
        TerminalBackend::kill(self, ()).is_ok()
    }

    /// True when the child runs on a real pseudo terminal (PTY/ConPTY), with
    /// echo and full terminal semantics. False for the piped fallback.
    pub fn is_real_pty(&self) -> bool {
        self.platform.is_real_pty()
    }

    /// Label of the backend in use (diagnostics/tests).
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn kind_label(&self) -> &'static str {
        self.platform.kind_label()
    }
}

/// `CommandSession` is the single-session `TerminalBackend`: it manages one
/// live process per instance (`Id = ()`), which is exactly the contract the
/// desktop needs — one persistent session per terminal window.
///
/// `spawn` re-hosts the instance on a fresh process (the previous session is
/// dropped, which kills and reaps it); `write`/`resize`/`kill` apply to the
/// owned session, and `poll` converts the drained stream into `TerminalEvent`s.
impl TerminalBackend for CommandSession {
    type Id = ();

    fn spawn(
        &mut self,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<Self::Id, String> {
        let fresh = CommandSession::spawn_with_args(program, args, cwd)?;
        *self = fresh;
        Ok(())
    }

    fn write(&mut self, _id: Self::Id, data: &[u8]) -> Result<(), String> {
        self.platform.write(data)
    }

    fn resize(&mut self, _id: Self::Id, cols: u16, rows: u16) -> Result<(), String> {
        self.platform.resize(cols, rows)
    }

    fn kill(&mut self, _id: Self::Id) -> Result<(), String> {
        self.platform.kill();
        Ok(())
    }

    fn poll(&mut self) -> Vec<TerminalEvent<Self::Id>> {
        let poll = CommandSession::poll(self);
        let mut events = Vec::new();
        for bytes in poll.outputs {
            events.push(TerminalEvent::Output { id: (), bytes });
        }
        if let Some(code) = poll.exit_code {
            events.push(TerminalEvent::Exit {
                id: (),
                code: Some(code),
            });
        }
        events
    }
}

// ── Platform selection ────────────────────────────────────────────────────────

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;

#[cfg(all(unix, not(target_os = "macos")))]
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
            for chunk in &session.poll().outputs {
                if chunk
                    .windows(b"first_12345".len())
                    .any(|w| w == b"first_12345")
                {
                    saw_first = true;
                }
            }
            if saw_first {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        session.write(b.as_bytes());
        let start = Instant::now();
        let mut saw_second = false;
        while start.elapsed() < Duration::from_secs(5) {
            for chunk in &session.poll().outputs {
                if chunk
                    .windows(b"second_67890".len())
                    .any(|w| w == b"second_67890")
                {
                    saw_second = true;
                }
            }
            if saw_second {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(
            saw_first && saw_second,
            "session did not survive both commands"
        );
    }

    #[test]
    fn terminal_backend_trait_contract_round_trips() {
        // The trait (ARCHITECTURE.md) must be usable as the backend boundary:
        // spawn with args, write, poll turning output into TerminalEvents,
        // resize and kill, all through a trait object.
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut backend: Box<dyn TerminalBackend<Id = ()>> =
            Box::new(CommandSession::spawn(default_shell(), &cwd).unwrap());

        // Re-host the session through the trait's spawn (explicit args).
        let empty_args: Vec<String> = Vec::new();
        backend
            .spawn(default_shell(), &empty_args, Some(&cwd))
            .unwrap();

        let marker = "manto_trait_marker_2027";
        #[cfg(windows)]
        let cmd = format!("echo {marker}\r\n");
        #[cfg(not(windows))]
        let cmd = format!("echo {marker}\n");
        backend.write((), cmd.as_bytes()).unwrap();

        let start = Instant::now();
        let mut saw_marker = false;
        let mut saw_exit_event = false;
        while start.elapsed() < Duration::from_secs(5) {
            for event in backend.poll() {
                match event {
                    TerminalEvent::Output { id: (), bytes } => {
                        if bytes.windows(marker.len()).any(|w| w == marker.as_bytes()) {
                            saw_marker = true;
                        }
                    }
                    TerminalEvent::Exit { id: (), .. } => saw_exit_event = true,
                }
            }
            if saw_marker {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(saw_marker, "trait-backed session did not echo the marker");
        let _ = backend.resize((), 80, 24);
        let _ = backend.kill(());
        // Kill guarantees an Exit event eventually; a trailing kill is fine even
        // if the shell already exited on its own.
        if !saw_exit_event {
            for _ in 0..50 {
                if backend
                    .poll()
                    .iter()
                    .any(|e| matches!(e, TerminalEvent::Exit { .. }))
                {
                    saw_exit_event = true;
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(saw_exit_event, "kill did not produce an Exit event");
    }
}
