use crate::terminal_backend::CommandSession;

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
    runner:           Option<CommandRunner>,
}

impl CommandEntry {
    pub fn completed(cmd: &str, cwd: &str, output_lines: Vec<String>) -> Self {
        Self {
            cwd: cwd.to_string(),
            command: cmd.trim().to_string(),
            output_lines,
            status: CommandStatus::Complete,
            runner: None,
        }
    }

    pub fn spawn(cmd: &str, cwd: &str) -> Self {
        let command = cmd.trim().to_string();
        match CommandSession::spawn(&command, cwd) {
            Ok(session) => Self {
                cwd: cwd.to_string(),
                command,
                output_lines: Vec::new(),
                status: CommandStatus::Running,
                runner: Some(CommandRunner::Session(session)),
            },
            Err(err) => Self {
                cwd: cwd.to_string(),
                command,
                output_lines: vec![err],
                status: CommandStatus::Complete,
                runner: None,
            },
        }
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
}

#[cfg(test)]
impl CommandEntry {
    pub fn fixture(command: &str, output_lines: &[&str], status: CommandStatus) -> Self {
        Self {
            cwd: String::new(),
            command: command.to_string(),
            output_lines: output_lines.iter().map(|line| (*line).to_string()).collect(),
            status,
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
}
