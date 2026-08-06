use std::mem;

use crate::cmd::{CommandEntry, tick_all};
use crate::terminal_backend::CommandSession;
use crate::window::Window;

// ── Estado de janela terminal ─────────────────────────────────────────────────

pub struct TerminalState {
    /// Persistent shell session for this terminal window.
    /// When Some, raw keyboard input is forwarded to the shell.
    pub shell_session: Option<CommandSession>,
    /// Accumulated raw output lines drained from the shell session.
    pub shell_lines:   Vec<String>,
    /// Historical command entries (displayed in the terminal content).
    pub commands:     Vec<CommandEntry>,
    pub cmd_input:    String,
    pub input_cursor: usize,
    pub panel_scroll: usize,
    pub path:         String,
    pub history_index: Option<usize>,
    pub history_draft: Option<String>,
    /// Prompt do REPL/aplicativo interativo em execução (ex.: ">>>" do Python).
    /// Quando Some, a janela oculta a barra " .> " e usa este prompt.
    pub repl_prompt:  Option<String>,
}

impl TerminalState {
    /// Detecta um prompt de REPL no fluxo (ex.: ">>>" do Python, "sqlite>",
    /// ">", etc.) para não exibi-lo como uma linha solta.
    fn looks_like_repl_prompt(line: &str) -> bool {
        let t = line.trim();
        !t.is_empty() && t.chars().count() <= 12 && t.ends_with('>')
    }

    pub fn new(path: String, commands: Vec<CommandEntry>) -> Self {
        Self {
            shell_session: None,
            shell_lines: Vec::new(),
            path,
            commands,
            cmd_input: String::new(),
            input_cursor: 0,
            panel_scroll: 0,
            history_index: None,
            history_draft: None,
            repl_prompt: None,
        }
    }

    /// Create a new TerminalState with an active shell session.
    pub fn with_shell(path: String, commands: Vec<CommandEntry>) -> Result<Self, String> {
        // Spawn a shell that will persist for the lifetime of this terminal window.
        // The shell runs as an interactive session.
        #[cfg(not(windows))]
        let shell_cmd = "/bin/sh".to_string();

        #[cfg(windows)]
        let shell_cmd = {
            if std::path::Path::new("pwsh.exe").exists() {
                "pwsh.exe".to_string()
            } else if std::path::Path::new("powershell.exe").exists() {
                "powershell.exe".to_string()
            } else {
                std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
            }
        };

        let mut session = CommandSession::spawn(&shell_cmd, &path)?;

        // Set initial terminal size
        let (cols, rows) = crate::os::size();
        session.resize(cols.saturating_sub(4), rows.saturating_sub(6));

        let shell_lines = Self::seed_shell_lines(&commands);
        Ok(Self {
            shell_session: Some(session),
            shell_lines,
            path,
            commands,
            cmd_input: String::new(),
            input_cursor: 0,
            panel_scroll: 0,
            history_index: None,
            history_draft: None,
            repl_prompt: None,
        })
    }

    /// Converte o histórico de comandos do dock em linhas de visualização do
    /// terminal, preservando o histórico ao transformar o dock em janela.
    fn seed_shell_lines(commands: &[CommandEntry]) -> Vec<String> {
        let mut lines = Vec::new();
        for entry in commands {
            if !entry.command.trim().is_empty() {
                lines.push(entry.command.clone());
            }
            lines.extend(entry.output_lines.iter().cloned());
        }
        lines
    }

    /// Avança um tick: drena saída de sessão e faz tick dos comandos.
    /// Retorna true se houve mudança.
    pub fn tick(&mut self) -> bool {
        let mut changed = tick_all(&mut self.commands);
        if let Some(ref mut session) = self.shell_session {
            let poll = session.poll();
            for line in poll.lines {
                changed |= self.ingest_output_line(line);
            }
            if poll.closed {
                self.repl_prompt = None;
            }
        }
        changed
    }

    /// Ingere uma linha da saída da sessão. Prompts de REPL são suprimidos do
    /// display e guardados como prefixo; demais linhas vão para `shell_lines`.
    pub(crate) fn ingest_output_line(&mut self, line: String) -> bool {
        if Self::looks_like_repl_prompt(&line) {
            self.repl_prompt = Some(line.trim().to_string());
        } else {
            self.push_shell_line(line);
        }
        true
    }

    /// Encerra o modo REPL (essencialmente quando o usuário envia EOF).
    pub fn clear_repl(&mut self) {
        self.repl_prompt = None;
    }

    /// Adiciona uma linha ao fluxo visual do shell, limitando o histórico.
    pub fn push_shell_line(&mut self, line: String) {
        self.shell_lines.push(line);
        const MAX_SHELL_LINES: usize = 2000;
        if self.shell_lines.len() > MAX_SHELL_LINES {
            let excess = self.shell_lines.len() - MAX_SHELL_LINES;
            self.shell_lines.drain(..excess);
        }
    }

    /// True se a janela de terminal possui uma sessão de shell ativa.
    pub fn has_session(&self) -> bool {
        self.shell_session.is_some()
    }
}

// ── Application ───────────────────────────────────────────────────────────────

pub struct Application {
    pub title:    String,
    pub display:  DisplayMode,
    pub desktop:  usize,
    /// Janelas de menu fecham ao perder o foco.
    pub is_menu:  bool,
    /// Presente em janelas de terminal; ausente em janelas comuns.
    pub terminal: Option<TerminalState>,
}

pub enum DisplayMode {
    Windowed(Window),
    Minimized(Window),
    Maximized { display: Window, saved: Window },
}

impl Application {
    pub fn windowed(title: impl Into<String>, window: Window) -> Self {
        Self { title: title.into(), display: DisplayMode::Windowed(window), desktop: 1, is_menu: false, terminal: None }
    }

    pub fn menu(title: impl Into<String>, window: Window) -> Self {
        Self { title: title.into(), display: DisplayMode::Windowed(window), desktop: 1, is_menu: true, terminal: None }
    }

    /// Cria uma janela de terminal com histórico de comandos pré-carregado e sessão de shell persistente.
    pub fn terminal_window(title: impl Into<String>, window: Window, path: String, commands: Vec<CommandEntry>) -> Self {
        match TerminalState::with_shell(path.clone(), commands.clone()) {
            Ok(ts) => Self {
                title:    title.into(),
                display:  DisplayMode::Windowed(window),
                desktop:  1,
                is_menu:  false,
                terminal: Some(ts),
            },
            Err(_) => {
                // Fallback to non-session mode if shell spawn fails
                Self {
                    title:    title.into(),
                    display:  DisplayMode::Windowed(window),
                    desktop:  1,
                    is_menu:  false,
                    terminal: Some(TerminalState::new(path, commands)),
                }
            }
        }
    }

    pub fn with_desktop(mut self, desktop: usize) -> Self {
        self.desktop = desktop;
        self
    }

    pub fn on_desktop(&self, desktop: usize) -> bool {
        self.desktop == desktop
    }

    pub fn window(&self) -> Option<&Window> {
        match &self.display {
            DisplayMode::Windowed(w)                  => Some(w),
            DisplayMode::Maximized { display: w, .. } => Some(w),
            _ => None,
        }
    }

    pub fn saved_window(&self) -> Option<&Window> {
        match &self.display {
            DisplayMode::Maximized { saved, .. } => Some(saved),
            _ => None,
        }
    }

    pub fn window_mut(&mut self) -> Option<&mut Window> {
        match &mut self.display {
            DisplayMode::Windowed(w)                  => Some(w),
            DisplayMode::Maximized { display: w, .. } => Some(w),
            _ => None,
        }
    }

    pub fn is_minimized(&self) -> bool {
        matches!(self.display, DisplayMode::Minimized(_))
    }

    pub fn is_maximized(&self) -> bool {
        matches!(self.display, DisplayMode::Maximized { .. })
    }

    pub fn minimize(&mut self) {
        let old = mem::replace(&mut self.display, DisplayMode::Minimized(Window::new(0, 0, 1, 1, 0)));
        self.display = match old {
            DisplayMode::Windowed(w) => DisplayMode::Minimized(w),
            other => other,
        };
    }

    pub fn restore(&mut self) {
        let old = mem::replace(&mut self.display, DisplayMode::Windowed(Window::new(0, 0, 1, 1, 0)));
        self.display = match old {
            DisplayMode::Minimized(w) => DisplayMode::Windowed(w),
            other => other,
        };
    }

    /// Maximiza a janela para ocupar o espaço útil da tela,
    /// preservando a geometria original para restauração.
    pub fn maximize(&mut self, screen_w: u16, screen_h: u16) {
        let old = mem::replace(&mut self.display, DisplayMode::Minimized(Window::new(0, 0, 1, 1, 0)));
        self.display = match old {
            DisplayMode::Windowed(w) => DisplayMode::Maximized {
                display: Window::new(
                    2, 1,
                    screen_w.saturating_sub(5),
                    screen_h.saturating_sub(4),
                    w.layer,
                ),
                saved: w,
            },
            other => other,
        };
    }

    pub fn restore_maximize(&mut self) {
        let old = mem::replace(&mut self.display, DisplayMode::Windowed(Window::new(0, 0, 1, 1, 0)));
        self.display = match old {
            DisplayMode::Maximized { saved, .. } => DisplayMode::Windowed(saved),
            other => other,
        };
    }

    pub fn set_window_geometry(&mut self, position_x: u16, position_y: u16, width: u16, height: u16) {
        let template = match &self.display {
            DisplayMode::Windowed(w) => Some(w),
            DisplayMode::Maximized { display, .. } => Some(display),
            DisplayMode::Minimized(_) => None,
        };

        let Some(template) = template else {
            return;
        };

        let mut win = Window::new(position_x, position_y, width, height, template.layer);
        win.minimizable = template.minimizable;
        win.closable = template.closable;
        win.draggable = template.draggable;
        win.resizable = template.resizable;
        win.content_w = template.content_w;
        win.content_h = template.content_h;
        win.scroll_x = template.scroll_x;
        win.scroll_y = template.scroll_y;
        self.display = DisplayMode::Windowed(win);
    }
}
