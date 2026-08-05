use crate::terminal_backend::CommandSession;

#[derive(Debug, PartialEq)]
pub enum CommandKind {
    Builtin,
    External,
}

enum CommandRunner {
    Session(CommandSession),
}

pub enum CommandStatus {
    Running,
    Complete,
}

pub struct CommandEntry {
    pub cwd:          String,
    pub command:      String,
    pub output_lines: Vec<String>,
    pub status:       CommandStatus,
    kind:             CommandKind,
    runner:           Option<CommandRunner>,
}

const BUILTINS_HELP: &[(&str, &str)] = &[
    ("cd <path>",        "Change the current directory"),
    ("pwd",              "Print the current working directory"),
    ("clear",            "Clear the terminal screen"),
    ("help",             "Show this help message"),
    ("exit",             "Close the focused terminal window"),
];

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
            cwd: cwd.to_string(),
            command: name.to_string(),
            output_lines,
            status: CommandStatus::Complete,
            kind,
            runner: None,
        }
    }

    pub fn spawn(cmd: &str, cwd: &str) -> Self {
        let command = cmd.trim().to_string();
        let first_word = command.split_whitespace().next().unwrap_or(&command);
        let kind = if is_builtin(first_word) {
            CommandKind::Builtin
        } else {
            CommandKind::External
        };

        match CommandSession::spawn(&command, cwd) {
            Ok(session) => Self {
                cwd: cwd.to_string(),
                command,
                output_lines: Vec::new(),
                status: CommandStatus::Running,
                kind,
                runner: Some(CommandRunner::Session(session)),
            },
            Err(err) => Self {
                cwd: cwd.to_string(),
                command,
                output_lines: vec![err],
                status: CommandStatus::Complete,
                kind,
                runner: None,
            },
        }
    }

    /// Run a builtin command and return the output lines.
    pub fn run_builtin(command: &str, cwd: &str) -> Vec<String> {
        let trimmed = command.trim();
        let first_word = trimmed.split_whitespace().next().unwrap_or(trimmed);

        match first_word {
            "pwd" => vec![cwd.to_string()],
            "clear" => Vec::new(),
            "help" => {
                let mut lines = vec!["Built-in commands:".to_string()];
                for (cmd, desc) in BUILTINS_HELP {
                    lines.push(format!("  {:20} {}", cmd, desc));
                }
                lines
            }
            "exit" => vec!["__EXIT__".to_string()],
            _ => Vec::new(),
        }
    }

    /// Check if this is a builtin command that exits the terminal.
    pub fn is_exit(&self) -> bool {
        self.kind == CommandKind::Builtin
            && self.command.trim().split_whitespace().next() == Some("exit")
    }

    /// Avança um tick. Retorna true se houve mudança.
    pub fn tick(&mut self) -> bool {
        match self.runner.take() {
            Some(CommandRunner::Session(mut session)) => {
                let poll = session.poll();
                let mut changed = false;

                for line in poll.lines {
                    if self.output_lines.is_empty() && line.trim().is_empty() {
                        continue;
                    }
                    self.output_lines.push(line);
                    changed = true;
                }

                if poll.closed {
                    if self.output_lines.is_empty() {
                        self.output_lines.push(match poll.exit_code.unwrap_or_default() {
                            0 => "complete".to_string(),
                            code => format!("exit {}", code),
                        });
                    }
                    self.status = CommandStatus::Complete;
                    changed = true;
                } else {
                    self.runner = Some(CommandRunner::Session(session));
                }

                changed
            }
            None => false,
        }
    }

    /// Kill a running external command. Returns true if killed.
    pub fn kill(&mut self) -> bool {
        match &mut self.runner {
            Some(CommandRunner::Session(session)) => session.kill(),
            None => false,
        }
    }

    /// Check if this is a running external command that can be killed.
    pub fn is_running_external(&self) -> bool {
        matches!(self.status, CommandStatus::Running)
            && matches!(self.kind, CommandKind::External)
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
            output_lines: output_lines.iter().map(|line| (*line).to_string()).collect(),
            status,
            kind: CommandKind::External,
            runner: None,
        }
    }
}

pub fn tick_all(commands: &mut Vec<CommandEntry>) -> bool {
    let mut changed = false;
    for e in commands.iter_mut() {
        if e.tick() { changed = true; }
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

        while start.elapsed() < Duration::from_secs(3) {
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
    fn builtin_pwd_returns_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let cwd_str = cwd.to_string_lossy().to_string();
        let output = CommandEntry::run_builtin("pwd", &cwd_str);
        assert_eq!(output, vec![cwd_str]);
    }

    #[test]
    fn builtin_clear_returns_empty() {
        let output = CommandEntry::run_builtin("clear", "/tmp");
        assert!(output.is_empty());
    }

    #[test]
    fn builtin_help_lists_commands() {
        let output = CommandEntry::run_builtin("help", "/tmp");
        assert!(output.len() > 1);
        assert!(output[0].contains("Built-in"));
    }

    #[test]
    fn builtin_exit_returns_exit_marker() {
        let output = CommandEntry::run_builtin("exit", "/tmp");
        assert_eq!(output, vec!["__EXIT__"]);
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
}
