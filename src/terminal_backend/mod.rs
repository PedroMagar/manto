use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Debug)]
pub enum TerminalUpdate {
    Line(String),
    Closed,
}

pub struct CommandSession {
    receiver: Receiver<TerminalUpdate>,
    platform: platform::PlatformCommand,
    closed_streams: usize,
}

pub struct CommandPoll {
    pub lines: Vec<String>,
    pub exit_code: Option<i32>,
    pub closed: bool,
}

impl CommandSession {
    pub fn spawn(command: &str, cwd: &str) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let platform = platform::spawn(command, cwd, tx)?;
        Ok(Self { receiver: rx, platform, closed_streams: 0 })
    }

    pub fn poll(&mut self) -> CommandPoll {
        let mut lines = Vec::new();

        loop {
            match self.receiver.try_recv() {
                Ok(TerminalUpdate::Line(line)) => lines.push(line),
                Ok(TerminalUpdate::Closed) => self.closed_streams += 1,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.closed_streams = 2;
                    break;
                }
            }
        }

        let exit_code = self.platform.try_wait();
        let closed = self.closed_streams >= 2 && exit_code.is_some();
        CommandPoll { lines, exit_code, closed }
    }
}

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

    #[test]
    fn spawn_echo_returns_output() {
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_string_lossy().to_string();
        let mut session = CommandSession::spawn("echo manto", &cwd).unwrap();
        let start = Instant::now();
        let mut lines = Vec::new();
        let mut exit_code = None;

        while start.elapsed() < Duration::from_secs(3) {
            let poll = session.poll();
            lines.extend(poll.lines);
            if poll.closed {
                exit_code = poll.exit_code;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(lines.iter().any(|line| line.contains("manto")), "output was: {lines:?}");
        assert_eq!(exit_code.unwrap_or_default(), 0);
    }

    #[test]
    fn failed_command_preserves_error_output() {
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_string_lossy().to_string();
        let mut session = CommandSession::spawn("this-command-does-not-exist", &cwd).unwrap();
        let start = Instant::now();
        let mut lines = Vec::new();
        let mut exit_code = None;

        while start.elapsed() < Duration::from_secs(3) {
            let poll = session.poll();
            lines.extend(poll.lines);
            if poll.closed {
                exit_code = poll.exit_code;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(!lines.is_empty(), "expected shell error output");
        assert_ne!(exit_code.unwrap_or_default(), 0);
    }

    #[test]
    fn dir_returns_output() {
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_string_lossy().to_string();
        let mut session = CommandSession::spawn("dir", &cwd).unwrap();
        let start = Instant::now();
        let mut lines = Vec::new();
        let mut exit_code = None;

        while start.elapsed() < Duration::from_secs(3) {
            let poll = session.poll();
            lines.extend(poll.lines);
            if poll.closed {
                exit_code = poll.exit_code;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(!lines.is_empty(), "output was: {lines:?}");
        assert_eq!(exit_code.unwrap_or_default(), 0);
    }

    #[test]
    fn ls_returns_output() {
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_string_lossy().to_string();
        let mut session = CommandSession::spawn("ls", &cwd).unwrap();
        let start = Instant::now();
        let mut lines = Vec::new();
        let mut exit_code = None;

        while start.elapsed() < Duration::from_secs(3) {
            let poll = session.poll();
            lines.extend(poll.lines);
            if poll.closed {
                exit_code = poll.exit_code;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(!lines.is_empty(), "output was: {lines:?}");
        assert_eq!(exit_code.unwrap_or_default(), 0);
    }
}
