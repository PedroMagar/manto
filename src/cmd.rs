enum CommandRunner {
    Timer(u32),
}

pub enum CommandStatus {
    Running,
    Complete,
}

pub struct CommandEntry {
    pub command:      String,
    pub output_lines: Vec<String>,
    pub status:       CommandStatus,
    runner:           Option<CommandRunner>,
}

impl CommandEntry {
    pub fn new(cmd: &str) -> Self {
        let parts: Vec<&str> = cmd.trim().split_whitespace().collect();
        if parts.len() == 2 && parts[0] == "timer" {
            if let Ok(n) = parts[1].parse::<u32>() {
                if n > 0 {
                    return Self {
                        command:      cmd.trim().to_string(),
                        output_lines: vec![format!("{}s", n)],
                        status:       CommandStatus::Running,
                        runner:       Some(CommandRunner::Timer(n)),
                    };
                }
            }
        }
        Self {
            command:      cmd.trim().to_string(),
            output_lines: vec!["command not found".to_string()],
            status:       CommandStatus::Complete,
            runner:       None,
        }
    }

    /// Avança um tick. Retorna true se houve mudança.
    pub fn tick(&mut self) -> bool {
        let Some(CommandRunner::Timer(rem)) = &self.runner else { return false; };
        let new_rem = rem.saturating_sub(1);
        if new_rem > 0 {
            self.output_lines.push(format!("{}s", new_rem));
            self.runner = Some(CommandRunner::Timer(new_rem));
        } else {
            self.output_lines.push("complete".to_string());
            self.status  = CommandStatus::Complete;
            self.runner  = None;
        }
        true
    }
}

pub fn tick_all(commands: &mut Vec<CommandEntry>) -> bool {
    let mut changed = false;
    for e in commands.iter_mut() {
        if e.tick() { changed = true; }
    }
    changed
}

