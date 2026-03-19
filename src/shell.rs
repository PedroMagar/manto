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

/// Constrói as linhas de conteúdo do painel com estrutura em árvore.
/// Retorna todas as linhas sem nenhum colapso — a rolagem e os indicadores
/// `├─ ...` são gerenciados pelo `draw_command_panel`.
#[allow(dead_code)]
pub fn build_panel_lines(commands: &[CommandEntry], path: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if !path.is_empty() {
        lines.push(path.to_string());
    }
    for entry in commands {
        let has_out = !entry.output_lines.is_empty();
        let cmd_pre = if has_out { "  ├─┬ " } else { "  ├─ " };
        lines.push(format!("{}{}", cmd_pre, entry.command));
        let all_out = &entry.output_lines;
        let last_idx = all_out.len().saturating_sub(1);
        for (j, out_line) in all_out.iter().enumerate() {
            let is_last = j == last_idx;
            let branch = if is_last { "└─ " } else { "│─ " };
            let suffix = if is_last {
                match entry.status {
                    CommandStatus::Running  => " (running)",
                    CommandStatus::Complete => "",
                }
            } else { "" };
            lines.push(format!("  │ {}{}{}", branch, out_line, suffix));
        }
    }
    lines
}
