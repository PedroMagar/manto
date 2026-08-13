use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum CommandKind {
    Builtin,
    External,
}

#[derive(Clone, Copy)]
pub enum CommandStatus {
    Running,
    Complete,
}

const BUILTINS_HELP: &[(&str, &str)] = &[
    ("cd <path>", "Change the current directory"),
    ("pwd", "Print the current working directory"),
    ("clear", "Clear the terminal screen"),
    ("help", "Show this help message"),
    ("exit", "Close the focused terminal window"),
];

pub struct CommandEntry {
    pub command: String,
    pub cwd: String,
    pub output_lines: Vec<String>,
    pub status: CommandStatus,
    kind: CommandKind,
    runner: Option<OneShot>,
}

impl Clone for CommandEntry {
    fn clone(&self) -> Self {
        Self {
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            output_lines: self.output_lines.clone(),
            status: self.status,
            kind: self.kind,
            runner: None,
        }
    }
}

impl CommandEntry {
    pub fn completed(cmd: &str, cwd: &str, output_lines: Vec<String>) -> Self {
        let name = cmd.trim();
        let first_word = name.split_whitespace().next().unwrap_or(name);
        let kind = if is_builtin(first_word) {
            CommandKind::Builtin
        } else {
            CommandKind::External
        };

        Self {
            command: name.to_string(),
            cwd: cwd.to_string(),
            output_lines,
            status: CommandStatus::Complete,
            kind,
            runner: None,
        }
    }

    /// Spawn a one-shot external command (used by the dock/typing mode).
    /// The output arrives via a background thread and is drained on `tick`.
    pub fn spawn(cmd: &str, cwd: &str) -> Self {
        let command = cmd.trim().to_string();
        let first_word = command.split_whitespace().next().unwrap_or(&command);
        let kind = if is_builtin(first_word) {
            CommandKind::Builtin
        } else {
            CommandKind::External
        };

        match OneShot::spawn(&command, cwd) {
            Ok(runner) => Self {
                command,
                cwd: cwd.to_string(),
                output_lines: Vec::new(),
                status: CommandStatus::Running,
                kind,
                runner: Some(runner),
            },
            Err(err) => Self {
                command,
                cwd: cwd.to_string(),
                output_lines: vec![err],
                status: CommandStatus::Complete,
                kind,
                runner: None,
            },
        }
    }

    /// Run a builtin command and return the output lines (for dock/typing mode).
    pub fn run_builtin(command: &str, cwd: &str) -> Vec<String> {
        let trimmed = command.trim();
        let first_word = trimmed.split_whitespace().next().unwrap_or(trimmed);

        match first_word {
            "pwd" => vec![cwd.to_string()],
            "clear" => Vec::new(),
            "help" => {
                let mut lines = vec!["Built-in commands:".to_string()];
                for (cmd, desc) in BUILTINS_HELP {
                    lines.push(format!("  {cmd:20} {desc}"));
                }
                lines
            }
            "exit" => vec!["__EXIT__".to_string()],
            _ => Vec::new(),
        }
    }

    /// Check if this is a builtin exit command.
    #[allow(dead_code)]
    pub fn is_exit(&self) -> bool {
        self.kind == CommandKind::Builtin && self.command.split_whitespace().next() == Some("exit")
    }

    /// Advance one tick. Returns true if anything changed.
    pub fn tick(&mut self) -> bool {
        let Some(mut runner) = self.runner.take() else {
            return false;
        };
        let (lines, exit_code, closed) = runner.poll();
        let mut changed = false;

        for line in lines {
            if self.output_lines.is_empty() && line.trim().is_empty() {
                continue;
            }
            self.output_lines.push(line);
            changed = true;
        }

        if closed {
            if self.output_lines.is_empty() {
                self.output_lines.push(match exit_code.unwrap_or_default() {
                    0 => "complete".to_string(),
                    code => format!("exit {code}"),
                });
            }
            self.status = CommandStatus::Complete;
            changed = true;
        } else {
            self.runner = Some(runner);
        }

        changed
    }

    /// Kill a running external command. Returns true if killed.
    pub fn kill(&mut self) -> bool {
        match &mut self.runner {
            Some(runner) => runner.kill(),
            None => false,
        }
    }

    /// Check if this is a running external command that can be killed.
    pub fn is_running_external(&self) -> bool {
        matches!(self.status, CommandStatus::Running) && matches!(self.kind, CommandKind::External)
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(name, "cd" | "pwd" | "clear" | "help" | "exit")
}

#[cfg(test)]
impl CommandEntry {
    pub fn fixture(command: &str, output_lines: &[&str], status: CommandStatus) -> Self {
        Self {
            cwd: String::new(),
            command: command.to_string(),
            output_lines: output_lines
                .iter()
                .map(|line| (*line).to_string())
                .collect(),
            status,
            kind: CommandKind::External,
            runner: None,
        }
    }
}

// ── One-shot command runner (dock) ────────────────────────────────────────────

enum RunnerUpdate {
    Line(String),
    Closed,
}

struct OneShot {
    child: Option<Child>,
    receiver: Receiver<RunnerUpdate>,
    closed_streams: usize,
}

/// Launch a shell to run `command` once, capturing its stdout/stderr lines.
fn spawn_process(command: &str, cwd: &str) -> Result<Child, String> {
    let spawn = |program: &str, args: &[&str]| -> Result<Child, String> {
        let mut cmd = Command::new(program);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to spawn {program}: {err}"))
    };

    #[cfg(windows)]
    {
        spawn("powershell.exe", &["-NoProfile", "-Command"])
            .or_else(|_| {
                let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
                spawn(&shell, &["/D", "/C"])
            })
            .or_else(|_| spawn("cmd.exe", &["/D", "/C"]))
    }

    #[cfg(not(windows))]
    {
        spawn("/bin/sh", &["-lc"])
    }
}

/// Read a byte stream fully, splitting on line breaks using lossy UTF-8
/// decoding so localized output (OEM/NT codepages) still reaches the UI.
fn read_lossy_lines<R: Read>(mut reader: R, tx: &mpsc::Sender<RunnerUpdate>) {
    let mut buf = [0u8; 4096];
    let mut residue = String::new();
    loop {
        let n = reader.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        residue.push_str(&String::from_utf8_lossy(&buf[..n]));
        while let Some(pos) = residue.find('\n') {
            let line: String = residue.drain(..=pos).collect();
            let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
            if !trimmed.is_empty() {
                let _ = tx.send(RunnerUpdate::Line(trimmed));
            }
        }
    }
    let tail = residue.trim().to_string();
    if !tail.is_empty() {
        let _ = tx.send(RunnerUpdate::Line(tail));
    }
    let _ = tx.send(RunnerUpdate::Closed);
}

impl OneShot {
    fn spawn(command: &str, cwd: &str) -> Result<Self, String> {
        let mut child = spawn_process(command, cwd)?;
        let (tx, rx) = mpsc::channel();

        if let Some(out) = child.stdout.take() {
            let tx = tx.clone();
            thread::spawn(move || {
                read_lossy_lines(out, &tx);
            });
        }
        if let Some(err) = child.stderr.take() {
            let tx = tx.clone();
            thread::spawn(move || {
                read_lossy_lines(err, &tx);
            });
        }

        Ok(Self {
            child: Some(child),
            receiver: rx,
            closed_streams: 0,
        })
    }

    fn poll(&mut self) -> (Vec<String>, Option<i32>, bool) {
        let mut lines = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(RunnerUpdate::Line(line)) => lines.push(line),
                Ok(RunnerUpdate::Closed) => self.closed_streams += 1,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.closed_streams = 2;
                    break;
                }
            }
        }
        let exit_code = self
            .child
            .as_mut()
            .and_then(|c| c.try_wait().ok().flatten())
            .map(|s| s.code().unwrap_or_default());
        let closed = self.closed_streams >= 2 && exit_code.is_some();
        (lines, exit_code, closed)
    }

    fn kill(&mut self) -> bool {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
        true
    }
}

impl Drop for OneShot {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn tick_all(commands: &mut [CommandEntry]) -> bool {
    let mut changed = false;
    for e in commands.iter_mut() {
        if e.tick() {
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn spawned_dir_keeps_output_lines() {
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_string_lossy().to_string();
        let mut cmd = CommandEntry::spawn("dir", &cwd);
        let start = Instant::now();

        while start.elapsed() < Duration::from_secs(5) {
            cmd.tick();
            if matches!(cmd.status, CommandStatus::Complete) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(!cmd.output_lines.is_empty(), "output was empty");
        assert!(cmd.output_lines.iter().any(|line| !line.trim().is_empty()));
    }

    #[test]
    fn entry_is_exit_detects_exit_command() {
        let entry = CommandEntry::completed("exit", "/tmp", Vec::new());
        assert!(entry.is_exit());

        let entry = CommandEntry::completed("pwd", "/tmp", Vec::new());
        assert!(!entry.is_exit());
    }

    #[test]
    fn entry_kind_is_builtin() {
        let entry = CommandEntry::completed("pwd", "/tmp", Vec::new());
        assert!(matches!(entry.kind, CommandKind::Builtin));

        let entry = CommandEntry::completed("ls", "/tmp", Vec::new());
        assert!(matches!(entry.kind, CommandKind::External));
    }

    #[test]
    fn lossy_reader_captures_non_utf8_lines() {
        let (tx, rx) = mpsc::channel();
        let bytes: Vec<u8> = vec![b'a', b'b', 0xFF, 0xFE, b'\n', b'c', b'd'];
        let handle = thread::spawn(move || {
            read_lossy_lines(&bytes[..], &tx);
        });
        handle.join().unwrap();

        let mut lines: Vec<String> = Vec::new();
        while let Ok(update) = rx.try_recv() {
            if let RunnerUpdate::Line(l) = update {
                lines.push(l);
            }
        }
        assert_eq!(lines.len(), 2, "expected two lines, got {lines:?}");
        assert!(
            lines[0].contains("ab"),
            "first line should contain ab: {lines:?}"
        );
        assert_eq!(lines[1], "cd");
    }

    #[test]
    fn unicode_command_survives_the_oneshot_round_trip() {
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_string_lossy().to_string();
        let mut cmd = CommandEntry::spawn("echo manto_çã_ñ", &cwd);
        let start = Instant::now();

        while start.elapsed() < Duration::from_secs(5) {
            cmd.tick();
            if matches!(cmd.status, CommandStatus::Complete) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        let joined = cmd.output_lines.join("\n");
        // Unix pipes are UTF-8 end to end; PowerShell 5.1 encodes piped
        // output in the OEM codepage, so on Windows only the ASCII part is
        // guaranteed to survive.
        #[cfg(unix)]
        assert!(
            joined.contains("manto_çã_ñ"),
            "accents must survive the round-trip: {joined:?}"
        );
        #[cfg(windows)]
        assert!(joined.contains("manto_"), "missing output: {joined:?}");
    }
}
