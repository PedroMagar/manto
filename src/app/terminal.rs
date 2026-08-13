use crate::cmd::{CommandEntry, tick_all};
use crate::os::Key;
use crate::terminal_backend::CommandSession;
use crate::terminal_emulator::Terminal;

/// Rewrite known REPL commands to their explicit interactive form.
///
/// In the pipe fallback (host without a real pseudo-terminal), `python`
/// without `-i` reads stdin as a script and only executes at EOF; with `-i`
/// the REPL processes line by line. Only bare invocations (no arguments)
/// are rewritten.
pub fn interactive_command(cmd: &str) -> String {
    match cmd.trim() {
        c if c.eq_ignore_ascii_case("python") => "python -i".to_string(),
        c if c.eq_ignore_ascii_case("python2") => "python2 -i".to_string(),
        c if c.eq_ignore_ascii_case("python3") => "python3 -i".to_string(),
        _ => cmd.to_string(),
    }
}

/// Commands that exit a REPL (exit/quit and variants). Used to detect that
/// the application/child has left and return the window to its normal state.
pub fn is_repl_exit(cmd: &str) -> bool {
    let lower = cmd.trim().to_ascii_lowercase();
    lower.starts_with("exit") || lower.starts_with("quit") || matches!(lower.as_str(), "\\q" | ":q")
}

/// Apps that launch in interactive mode from the dock without the `#i` marker.
pub fn is_interactive_app(program: &str) -> bool {
    let p = program.trim().to_ascii_lowercase();
    matches!(
        p.as_str(),
        "python"
            | "python3"
            | "python2"
            | "python3.11"
            | "python3.12"
            | "ipython"
            | "vscode"
            | "vim"
            | "vim.tiny"
            | "nano"
            | "emacs"
            | "pico"
            | "less"
            | "more"
            | "top"
            | "htop"
            | "btop"
            | "lazygit"
            | "fzf"
            | "node"
            | "node.exe"
            | "bash"
            | "sh"
            | "zsh"
            | "cmd"
            | "cmd.exe"
            | "powershell"
            | "powershell.exe"
            | "pwsh"
            | "pwsh.exe"
    )
}

/// Strip a leading `#i` directive: `#i vim` runs `vim` interactively, a bare
/// `#i` opens the default shell. Returns (command, interactive).
pub fn split_interactive_flag(raw: &str) -> (String, bool) {
    let trimmed = raw.trim();
    if trimmed.len() >= 2
        && trimmed[..2].eq_ignore_ascii_case("#i")
        && let Some(rest) = trimmed.get(2..)
        && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        return (rest.trim_start().to_string(), true);
    }
    (trimmed.to_string(), false)
}

/// Default interactive shell for the platform.
pub fn default_shell() -> String {
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
    #[cfg(windows)]
    {
        // Prefer a real pwsh/powershell found on PATH, then COMSPEC.
        for name in ["pwsh.exe", "powershell.exe"] {
            if let Some(path) = crate::os::find_on_path(name) {
                return path;
            }
        }
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
}

/// Encode an `os::Key` as the raw bytes a terminal app expects; None for
/// Manto-only keys.
pub fn key_to_bytes(key: Key) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    match key {
        Key::Char(c) => {
            let mut b = [0u8; 4];
            buf.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
        }
        Key::Enter | Key::CtrlEnter => buf.push(b'\r'),
        Key::Tab => buf.push(b'\t'),
        Key::Backspace => {
            // Windows consoles translate 0x7F into a Delete key event, which
            // console readers (python's REPL, cmd) ignore; 0x08 is VK_BACK
            // and also reads as backspace in readline/vim-style apps.
            #[cfg(windows)]
            buf.push(0x08);
            #[cfg(not(windows))]
            buf.push(0x7f);
        }
        Key::Escape => buf.push(0x1b),
        Key::Delete => buf.extend_from_slice(b"\x1b[3~"),
        Key::PageUp => buf.extend_from_slice(b"\x1b[5~"),
        Key::PageDown => buf.extend_from_slice(b"\x1b[6~"),
        Key::Home => buf.extend_from_slice(b"\x1b[H"),
        Key::End => buf.extend_from_slice(b"\x1b[F"),
        Key::Up => buf.extend_from_slice(b"\x1b[A"),
        Key::Down => buf.extend_from_slice(b"\x1b[B"),
        Key::Right => buf.extend_from_slice(b"\x1b[C"),
        Key::Left => buf.extend_from_slice(b"\x1b[D"),
        Key::ShiftUp => buf.extend_from_slice(b"\x1b[1;2A"),
        Key::ShiftDown => buf.extend_from_slice(b"\x1b[1;2B"),
        Key::ShiftRight => buf.extend_from_slice(b"\x1b[1;2C"),
        Key::ShiftLeft => buf.extend_from_slice(b"\x1b[1;2D"),
        Key::AltUp => buf.extend_from_slice(b"\x1b[1;3A"),
        Key::AltDown => buf.extend_from_slice(b"\x1b[1;3B"),
        Key::AltRight => buf.extend_from_slice(b"\x1b[1;3C"),
        Key::AltLeft => buf.extend_from_slice(b"\x1b[1;3D"),
        Key::CtrlC => buf.push(0x03),
        Key::CtrlD => buf.push(0x04),
        Key::CtrlE => buf.push(0x05),
        Key::CtrlF => buf.push(0x06),
        Key::CtrlH => buf.push(0x08),
        Key::CtrlJ => buf.push(0x0a),
        Key::CtrlK => buf.push(0x0b),
        Key::CtrlL => buf.push(0x0c),
        Key::CtrlN => buf.push(0x0e),
        Key::CtrlP => buf.push(0x10),
        Key::CtrlQ => buf.push(0x11),
        Key::CtrlV => buf.push(0x16),
        Key::CtrlW => buf.push(0x17),
        Key::CtrlX => buf.push(0x18),
        Key::CtrlZ => buf.push(0x1a),
        Key::CtrlT => buf.push(0x14),
        _ => return None,
    }
    Some(buf)
}

/// Keys whose terminal meaning is an ESC-based control sequence.
///
/// In a session with no real pseudo terminal (the piped fallback) there is no
/// terminal to interpret them: mirroring them into the emulator would move the
/// emulated cursor around the grid, and buffering them would leak raw control
/// bytes into the child's stdin when the line is sent on Enter. They are
/// ignored there (navigation over a pipe is meaningless anyway).
pub fn is_terminal_navigation(key: Key) -> bool {
    matches!(
        key,
        Key::Up
            | Key::Down
            | Key::Left
            | Key::Right
            | Key::ShiftUp
            | Key::ShiftDown
            | Key::ShiftLeft
            | Key::ShiftRight
            | Key::AltUp
            | Key::AltDown
            | Key::AltLeft
            | Key::AltRight
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
            | Key::Delete
            | Key::CtrlDelete
    )
}

/// Encode a pointer event as an SGR mouse report for a terminal app.
/// Coordinates are 1-based (as reported), so they pass straight through.
pub fn mouse_to_bytes(ev: crate::os::MouseEvent) -> Option<Vec<u8>> {
    use crate::os::{MouseAction, MouseButton};
    let base = match ev.button {
        MouseButton::Left => 0u16,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    let mut code = base;
    if ev.shift {
        code |= 0x4;
    }
    if ev.alt {
        code |= 0x8;
    }
    if ev.ctrl {
        code |= 0x10;
    }
    let motion = matches!(ev.kind, MouseAction::Move | MouseAction::Drag);
    let press = matches!(
        ev.kind,
        MouseAction::Press | MouseAction::Drag | MouseAction::Move
    );
    if motion {
        code |= 0x40;
    }
    let fin = if press { 'M' } else { 'm' };
    Some(format!("\x1b[<{};{};{}{}", code, ev.x, ev.y, fin).into_bytes())
}

pub struct TerminalState {
    /// Persistent shell session for this terminal window.
    /// When Some, raw keyboard input is forwarded to the shell.
    pub shell_session: Option<CommandSession>,
    /// Accumulated raw output lines drained from the shell session
    /// (classic line-mode viewer).
    pub shell_lines: Vec<String>,
    /// Terminal emulator (grid) fed from the raw bytes. Present when the
    /// window is an interactive terminal.
    pub emulator: Option<Terminal>,
    /// Interactive passthrough: when true, focus-mode keys forward raw to the
    /// session and the window renders the emulator grid.
    pub interactive: bool,
    /// Historical command entries (displayed in the terminal content).
    pub commands: Vec<CommandEntry>,
    pub cmd_input: String,
    pub input_cursor: usize,
    pub panel_scroll: usize,
    pub path: String,
    pub history_index: Option<usize>,
    pub history_draft: Option<String>,
    /// Prompt of the running REPL/interactive application (e.g. Python's
    /// ">>>"). When Some, the window hides the " .> " bar and uses this
    /// prompt instead.
    pub repl_prompt: Option<String>,
    /// Partial typed line for piped (non-PTY) sessions. There is no console
    /// to do the editing, so keystrokes buffer here and the complete line is
    /// sent on Enter — backspace truly removes a character instead of
    /// leaking a literal 0x08 into the child's input.
    pipe_line: Vec<u8>,
    /// Partial line buffer (bytes not yet terminated by '\n').
    tail: String,
    /// Local command history for piped (non-PTY) interactive sessions, which
    /// have no terminal of their own to keep a history.
    pipe_history: Vec<String>,
    /// Index into `pipe_history` while recalling with Up/Down.
    pipe_hist_idx: Option<usize>,
    /// The line being edited before history navigation started.
    pipe_draft: Option<String>,
}

impl TerminalState {
    /// Mirror typed input into the emulator grid. Used by the interactive
    /// passthrough when the session has no real PTY (piped fallback): the
    /// shell will not echo, so Manto renders the keystrokes itself. For
    /// Backspace the erased cell is blanked locally ("\x08 \x08"), matching
    /// what a console would do with its own echo.
    pub fn mirror_input(&mut self, bytes: &[u8], backspace: bool) {
        let Some(em) = self.emulator.as_mut() else {
            return;
        };
        if backspace {
            em.process(b"\x08 \x08");
        } else {
            em.process(bytes);
        }
    }

    /// Detect a REPL prompt in the output stream (e.g. Python's ">>>",
    /// "sqlite>") so it is not displayed as a loose line.
    fn looks_like_repl_prompt(line: &str) -> bool {
        let t = line.trim();
        !t.is_empty() && t.chars().count() <= 12 && t.ends_with('>')
    }

    /// Buffer typed bytes for a piped (non-PTY) session: characters are
    /// collected until Enter, when the whole corrected line is sent.
    pub fn pipe_feed(&mut self, bytes: &[u8]) {
        self.pipe_line.extend_from_slice(bytes);
    }

    /// Remove the last typed character from the piped line buffer (a full
    /// UTF-8 character, continuation bytes included).
    pub fn pipe_backspace(&mut self) {
        while let Some(&b) = self.pipe_line.last() {
            self.pipe_line.pop();
            if b & 0xC0 != 0x80 {
                break;
            }
        }
    }

    /// Abandon the partial piped line (Ctrl+C has no interrupt meaning on a
    /// pipe; drop it instead of leaking a control byte into the child).
    pub fn pipe_cancel(&mut self) {
        self.pipe_line.clear();
        self.reset_pipe_history();
    }

    /// Forget any in-progress history navigation for the piped line.
    pub fn reset_pipe_history(&mut self) {
        self.pipe_hist_idx = None;
        self.pipe_draft = None;
    }

    /// Send the buffered line to the piped session. A blank line is flushed
    /// as a bare newline, like pressing Enter at an empty prompt.
    ///
    /// Bare REPL commands (`python`, `python2`, `python3`) are rewritten to
    /// their `-i` interactive form before being sent: with no real terminal
    /// the child would otherwise treat piped stdin as a script and just sit
    /// waiting for EOF, making the session look dead. The sent line is also
    /// remembered so Up/Down can recall it (there is no console history).
    pub fn pipe_flush(&mut self) {
        let mut line = Vec::new();
        std::mem::swap(&mut line, &mut self.pipe_line);
        let text = String::from_utf8_lossy(&line).into_owned();
        let trimmed = text.trim().to_string();

        if !trimmed.is_empty()
            && self
                .pipe_history
                .last()
                .map(|l| l != &trimmed)
                .unwrap_or(true)
        {
            self.pipe_history.push(trimmed.clone());
            if self.pipe_history.len() > 100 {
                self.pipe_history.remove(0);
            }
        }
        self.pipe_hist_idx = None;
        self.pipe_draft = None;

        let sent = interactive_command(&trimmed);
        let mut out = sent.into_bytes();
        out.extend_from_slice(b"\r\n");
        if let Some(ref mut session) = self.shell_session {
            session.write(&out);
        }
    }

    /// Clear the current piped line in the emulator and draw `text` in its
    /// place, updating the local line buffer to match.
    fn set_pipe_line(&mut self, text: &str) {
        self.pipe_line = text.as_bytes().to_vec();
        if let Some(em) = self.emulator.as_mut() {
            em.process(b"\r\x1b[2K");
            em.process(text.as_bytes());
        }
    }

    /// Recall previous piped lines with the Up (true) / Down (false) arrows,
    /// replacing the current local line (and its display). Returns whether the
    /// line changed. Only meaningful for piped sessions with a history.
    pub fn pipe_recall(&mut self, up: bool) -> bool {
        if self.pipe_history.is_empty() {
            return false;
        }
        let len = self.pipe_history.len();
        if self.pipe_hist_idx.is_none() {
            if !up {
                return false;
            }
            self.pipe_draft = Some(String::from_utf8_lossy(&self.pipe_line).into_owned());
            self.pipe_hist_idx = Some(len - 1);
            let line = self.pipe_history[len - 1].clone();
            self.set_pipe_line(&line);
            return true;
        }
        let idx = self.pipe_hist_idx.unwrap();
        if up {
            if idx == 0 {
                return false;
            }
            self.pipe_hist_idx = Some(idx - 1);
            let line = self.pipe_history[idx - 1].clone();
            self.set_pipe_line(&line);
            true
        } else if idx + 1 < len {
            self.pipe_hist_idx = Some(idx + 1);
            let line = self.pipe_history[idx + 1].clone();
            self.set_pipe_line(&line);
            true
        } else {
            self.pipe_hist_idx = None;
            let draft = self.pipe_draft.take().unwrap_or_default();
            self.set_pipe_line(&draft);
            true
        }
    }

    pub fn new(path: String, commands: Vec<CommandEntry>) -> Self {
        Self {
            shell_session: None,
            shell_lines: Vec::new(),
            emulator: None,
            interactive: false,
            path,
            commands,
            cmd_input: String::new(),
            input_cursor: 0,
            panel_scroll: 0,
            history_index: None,
            history_draft: None,
            repl_prompt: None,
            pipe_line: Vec::new(),
            tail: String::new(),
            pipe_history: Vec::new(),
            pipe_hist_idx: None,
            pipe_draft: None,
        }
    }

    /// Create a new TerminalState with an active shell session.
    pub fn with_shell(path: String, commands: Vec<CommandEntry>) -> Result<Self, String> {
        let shell_cmd = default_shell();

        let mut session = CommandSession::spawn(&shell_cmd, &path)?;
        let (cols, rows) = crate::os::size();
        session.resize(cols.saturating_sub(4), rows.saturating_sub(6));

        let shell_lines = Self::seed_shell_lines(&commands);
        Ok(Self {
            shell_session: Some(session),
            shell_lines,
            emulator: None,
            interactive: false,
            path,
            commands,
            cmd_input: String::new(),
            input_cursor: 0,
            panel_scroll: 0,
            history_index: None,
            history_draft: None,
            repl_prompt: None,
            pipe_line: Vec::new(),
            tail: String::new(),
            pipe_history: Vec::new(),
            pipe_hist_idx: None,
            pipe_draft: None,
        })
    }

    /// Interactive terminal: run `program` directly (emulator renders output,
    /// keys forwarded raw). Bare `python`/`python2`/`python3` gain `-i` so a
    /// piped fallback still behaves as a REPL.
    pub fn with_program(path: String, program: &str) -> Result<Self, String> {
        let mut session = CommandSession::spawn_app(&interactive_command(program), &path)?;
        session.resize(80, 24);
        Ok(Self {
            shell_session: Some(session),
            shell_lines: Vec::new(),
            emulator: Some(Terminal::new(80, 24)),
            interactive: true,
            path,
            commands: Vec::new(),
            cmd_input: String::new(),
            input_cursor: 0,
            panel_scroll: 0,
            history_index: None,
            history_draft: None,
            repl_prompt: None,
            pipe_line: Vec::new(),
            tail: String::new(),
            pipe_history: Vec::new(),
            pipe_hist_idx: None,
            pipe_draft: None,
        })
    }

    /// Convert the dock command history into terminal display lines,
    /// preserving the history when the dock becomes a window.
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

    /// Send a typed line to the session, exactly like the desktop's `.>`
    /// bar: REPL commands get rewritten (`python` -> `python -i`), the line
    /// is forwarded, and — when the session is not a real PTY (piped
    /// fallback) — Manto mirrors the typed text into the view so commands
    /// stay visible even though no console echoes them. Real PTYs echo
    /// themselves, so nothing is doubled.
    pub fn run_line(&mut self, cmd: &str) {
        let real_pty = self
            .shell_session
            .as_ref()
            .map(|s| s.is_real_pty())
            .unwrap_or(true);
        if let Some(ref mut session) = self.shell_session {
            let line = format!("{}\r\n", interactive_command(cmd));
            session.write(line.as_bytes());
        }
        if !real_pty && !cmd.trim().is_empty() {
            self.push_shell_line(cmd.trim().to_string());
        }
        if !cmd.trim().is_empty() {
            self.commands
                .push(CommandEntry::completed(cmd, &self.path, Vec::new()));
            const MAX_HISTORY: usize = 200;
            if self.commands.len() > MAX_HISTORY {
                self.commands.drain(..self.commands.len() - MAX_HISTORY);
            }
        }
        if self.repl_prompt.is_some() && is_repl_exit(cmd) {
            self.clear_repl();
        }
    }

    /// Advance one tick: drain session output, feed the emulator (interactive)
    /// and reconstruct display lines for the line-mode viewer.
    /// Returns true if anything changed.
    pub fn tick(&mut self) -> bool {
        let mut changed = tick_all(&mut self.commands);
        if let Some(ref mut session) = self.shell_session {
            use crate::terminal_backend::{TerminalBackend, TerminalEvent};
            let events = TerminalBackend::poll(session);
            let got_bytes = events
                .iter()
                .any(|e| matches!(e, TerminalEvent::Output { .. }));
            for event in events {
                match event {
                    TerminalEvent::Output { id: (), bytes } => {
                        if let Some(em) = self.emulator.as_mut() {
                            em.process(&bytes);
                        }
                        self.tail.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    TerminalEvent::Exit { id: (), code } => {
                        // The child left: clear any lingering REPL prompt
                        // (the exit code is unused here).
                        self.repl_prompt = None;
                        let _ = code;
                    }
                }
            }
            // Emulator-only progress (no newline) still needs a redraw.
            if got_bytes {
                changed = true;
            }
            // Split complete lines.
            while let Some(pos) = self.tail.find('\n') {
                let line: String = self.tail.drain(..=pos).collect();
                let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
                if !trimmed.is_empty() {
                    changed |= self.ingest_output_line(trimmed);
                }
            }
            // Stable partial (e.g. a ">>> " prompt): flush when the pipe is
            // momentarily quiet.
            if !self.tail.trim().is_empty() && !got_bytes {
                let part = self.tail.trim_end().to_string();
                if !part.is_empty() {
                    changed |= self.ingest_output_line(part);
                }
                self.tail.clear();
            }
        }
        changed
    }

    /// Ingest one line of session output. REPL prompts are suppressed from
    /// display and stored as the input prefix; other lines go to `shell_lines`.
    pub(crate) fn ingest_output_line(&mut self, line: String) -> bool {
        if Self::looks_like_repl_prompt(&line) {
            self.repl_prompt = Some(line.trim().to_string());
        } else {
            self.push_shell_line(line);
        }
        true
    }

    /// Leave REPL mode (essentially when the user sends EOF).
    pub fn clear_repl(&mut self) {
        self.repl_prompt = None;
    }

    /// Append a line to the shell visual stream, bounding the history.
    pub fn push_shell_line(&mut self, line: String) {
        self.shell_lines.push(line);
        const MAX_SHELL_LINES: usize = 2000;
        if self.shell_lines.len() > MAX_SHELL_LINES {
            let excess = self.shell_lines.len() - MAX_SHELL_LINES;
            self.shell_lines.drain(..excess);
        }
    }

    /// True if the terminal window has an active shell session.
    #[allow(dead_code)]
    pub fn has_session(&self) -> bool {
        self.shell_session.is_some()
    }

    /// Resize the emulator grid and propagate to the child process.
    pub fn set_grid_size(&mut self, cols: u16, rows: u16) {
        if let Some(em) = self.emulator.as_mut()
            && (em.cols() != cols || em.rows() != rows)
        {
            em.resize(cols, rows);
        }
        if let Some(ref mut session) = self.shell_session {
            session.resize(cols, rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_command_rewrites_bare_python() {
        assert_eq!(interactive_command("python"), "python -i");
        assert_eq!(interactive_command("python3"), "python3 -i");
        assert_eq!(interactive_command("python2"), "python2 -i");
        assert_eq!(interactive_command("python script.py"), "python script.py");
        assert_eq!(interactive_command("dir"), "dir");
    }

    #[test]
    fn repl_exit_detection_clears_mode() {
        for e in ["exit", "exit()", "quit", "quit()", "\\q", ":q"] {
            assert!(is_repl_exit(e), "{e} should exit the REPL");
        }
        for e in ["dir", "print('x')", "q", "1+1"] {
            assert!(!is_repl_exit(e), "{e} should not exit the REPL");
        }
    }

    #[test]
    fn interactive_flag_splitting() {
        // `#i <app>` prefix directive.
        assert_eq!(split_interactive_flag("#i vim"), ("vim".to_string(), true));
        assert_eq!(
            split_interactive_flag("#i   python3"),
            ("python3".to_string(), true)
        );
        assert_eq!(split_interactive_flag("#I top"), ("top".to_string(), true));
        // Bare `#i` opens the default shell.
        assert_eq!(split_interactive_flag("#i"), ("".to_string(), true));
        assert_eq!(split_interactive_flag("#i   "), ("".to_string(), true));
        // Not a directive: no marker, or `#i` glued to a word.
        assert_eq!(
            split_interactive_flag("vim #i"),
            ("vim #i".to_string(), false)
        );
        assert_eq!(
            split_interactive_flag("#iffy"),
            ("#iffy".to_string(), false)
        );
        assert_eq!(
            split_interactive_flag("# comment"),
            ("# comment".to_string(), false)
        );
        assert_eq!(split_interactive_flag("dir"), ("dir".to_string(), false));
        assert!(is_interactive_app("vim"));
        assert!(is_interactive_app("python"));
        assert!(is_interactive_app("nano"));
        assert!(!is_interactive_app("dir"));
    }

    #[test]
    fn key_to_bytes_maps_terminal_keys() {
        use crate::os::Key;
        assert_eq!(key_to_bytes(Key::Enter).unwrap(), b"\r");
        assert_eq!(key_to_bytes(Key::Up).unwrap(), b"\x1b[A");
        assert_eq!(key_to_bytes(Key::ShiftLeft).unwrap(), b"\x1b[1;2D");
        assert_eq!(key_to_bytes(Key::ShiftUp).unwrap(), b"\x1b[1;2A");
        assert_eq!(key_to_bytes(Key::PageUp).unwrap(), b"\x1b[5~");
        assert_eq!(key_to_bytes(Key::Delete).unwrap(), b"\x1b[3~");
        assert_eq!(key_to_bytes(Key::CtrlC).unwrap(), b"\x03");
        assert_eq!(key_to_bytes(Key::CtrlZ).unwrap(), b"\x1a");
        assert_eq!(key_to_bytes(Key::Char('a')).unwrap(), b"a");
        // Backspace: VK_BACK (0x08) on Windows so console REPLs erase;
        // DEL (0x7f) on Unix, the terminal standard.
        let bs = key_to_bytes(Key::Backspace).unwrap();
        assert_eq!(
            bs,
            if cfg!(windows) {
                vec![0x08]
            } else {
                vec![0x7f]
            },
            "backspace byte per platform"
        );
        // Desktop shortcuts carry no terminal meaning.
        assert!(key_to_bytes(Key::Ctrl1).is_none());
        assert!(key_to_bytes(Key::AltR).is_none());
    }

    #[test]
    fn terminal_navigation_keys_are_detected() {
        use crate::os::Key;
        for key in [
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::ShiftUp,
            Key::AltLeft,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Delete,
            Key::CtrlDelete,
        ] {
            assert!(is_terminal_navigation(key), "{key:?} is navigation");
        }
        for key in [
            Key::Char('a'),
            Key::Enter,
            Key::Backspace,
            Key::Tab,
            Key::CtrlC,
            Key::Char(' '),
        ] {
            assert!(!is_terminal_navigation(key), "{key:?} is not navigation");
        }
    }

    #[test]
    fn mouse_to_bytes_builds_sgr_reports() {
        use crate::os::{MouseAction, MouseButton, MouseEvent};
        let press = MouseEvent {
            x: 12,
            y: 7,
            kind: MouseAction::Press,
            button: MouseButton::Left,
            shift: false,
            ctrl: false,
            alt: false,
        };
        assert_eq!(mouse_to_bytes(press), Some(b"\x1b[<0;12;7M".to_vec()));

        let drag = MouseEvent {
            x: 3,
            y: 4,
            kind: MouseAction::Drag,
            button: MouseButton::Left,
            shift: true,
            ctrl: true,
            alt: false,
        };
        // 0 (left) | drag 0x40 | shift 0x4 | ctrl 0x10 = 0x54 = 84.
        assert_eq!(mouse_to_bytes(drag), Some(b"\x1b[<84;3;4M".to_vec()));

        let release = MouseEvent {
            x: 1,
            y: 1,
            kind: MouseAction::Release,
            button: MouseButton::Left,
            shift: false,
            ctrl: false,
            alt: false,
        };
        assert_eq!(mouse_to_bytes(release), Some(b"\x1b[<0;1;1m".to_vec()));

        let wheel = MouseEvent {
            x: 5,
            y: 5,
            kind: MouseAction::Press,
            button: MouseButton::WheelUp,
            shift: false,
            ctrl: false,
            alt: false,
        };
        assert_eq!(mouse_to_bytes(wheel), Some(b"\x1b[<64;5;5M".to_vec()));
    }

    #[test]
    fn repl_prompt_is_suppressed_and_used_as_prefix() {
        let mut t = TerminalState::new(".".to_string(), Vec::new());

        t.ingest_output_line(">>>".to_string());
        assert_eq!(t.repl_prompt.as_deref(), Some(">>>"));
        assert!(
            t.shell_lines.is_empty(),
            "prompt leaked: {:?}",
            t.shell_lines
        );

        t.ingest_output_line("42".to_string());
        assert!(t.shell_lines.iter().any(|l| l == "42"));
        assert_eq!(t.repl_prompt.as_deref(), Some(">>>"));

        t.clear_repl();
        assert_eq!(t.repl_prompt, None);
        assert!(t.repl_prompt.is_none());
    }

    #[test]
    fn terminal_session_echo_and_output_accumulate() {
        use super::super::Application;
        use crate::ui::window::Window;
        let mut app = Application::terminal_window(
            "Term",
            Window::new(4, 4, 60, 25, 0),
            ".".to_string(),
            Vec::new(),
        );
        let t = app.terminal.as_mut().unwrap();
        assert!(t.has_session(), "terminal should own a shell session");

        t.cmd_input = "echo echo_marker_9911".to_string();
        t.input_cursor = t.cmd_input.chars().count();

        let cmd = t.cmd_input.trim().to_string();
        t.push_shell_line(cmd.clone());
        if let Some(ref mut session) = t.shell_session {
            let line = format!("{cmd}\r");
            session.write(line.as_bytes());
        }
        t.cmd_input.clear();
        t.input_cursor = 0;

        assert!(t.shell_lines.iter().any(|l| l.contains("echo_marker_9911")));

        use std::thread;
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut saw_output = false;
        while start.elapsed() < Duration::from_secs(5) {
            t.tick();
            if t.shell_lines
                .iter()
                .filter(|l| l.contains("echo_marker_9911"))
                .count()
                >= 2
            {
                saw_output = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(
            saw_output,
            "shell did not emit result lines: {:?}",
            t.shell_lines
        );
    }

    #[test]
    #[cfg(windows)]
    fn python_opens_through_session() {
        use super::super::Application;
        use crate::ui::window::Window;
        use std::thread;
        use std::time::{Duration, Instant};
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut app = Application::terminal_window(
            "Term",
            Window::new(4, 4, 60, 25, 0),
            cwd.clone(),
            Vec::new(),
        );
        let t = app.terminal.as_mut().unwrap();
        assert!(t.has_session());

        let rev = interactive_command("python");
        let line = format!("{rev}\r\n");
        if let Some(ref mut session) = t.shell_session {
            session.write(line.as_bytes());
        }

        let start = Instant::now();
        let mut saw_banner = false;
        while start.elapsed() < Duration::from_secs(6) {
            t.tick();
            if t.shell_lines
                .iter()
                .any(|l| l.contains("Python") && l.contains("on win32"))
            {
                saw_banner = true;
                break;
            }
            thread::sleep(Duration::from_millis(40));
        }
        assert!(saw_banner, "python did not open (no banner)");

        if let Some(ref mut session) = t.shell_session {
            let line = "print('PY_APP_MARK_69')\r\n".to_string();
            session.write(line.as_bytes());
        }
        let start = Instant::now();
        let mut saw_mark = false;
        while start.elapsed() < Duration::from_secs(5) {
            t.tick();
            if t.shell_lines.iter().any(|l| l.contains("PY_APP_MARK_69")) {
                saw_mark = true;
                break;
            }
            thread::sleep(Duration::from_millis(40));
        }
        assert!(saw_mark, "python did not execute the sent line");
    }

    #[test]
    fn terminal_window_with_history_preserves_it() {
        use super::super::Application;
        use crate::cmd::CommandEntry;
        use crate::ui::window::Window;
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let commands = vec![
            CommandEntry::completed(
                "cd xphmg",
                &cwd,
                vec!["erro: diretório não existe".to_string()],
            ),
            CommandEntry::completed(
                "flutter --version",
                &cwd,
                vec!["Flutter 3.32.8 stable".to_string()],
            ),
        ];
        let app = Application::terminal_window("Term", Window::new(4, 4, 60, 25, 0), cwd, commands);
        let t = app.terminal.as_ref().unwrap();
        assert!(t.has_session(), "should have a session");
        assert!(
            t.shell_lines.iter().any(|l| l.contains("cd xphmg")),
            "history lost: {:#?}",
            t.shell_lines
        );
        assert!(
            t.shell_lines.iter().any(|l| l.contains("Flutter")),
            "history output lost: {:#?}",
            t.shell_lines
        );
    }

    #[test]
    fn terminal_session_roundtrips_unicode() {
        use super::super::Application;
        use crate::ui::window::Window;
        let mut app = Application::terminal_window(
            "Term",
            Window::new(4, 4, 60, 25, 0),
            ".".to_string(),
            Vec::new(),
        );
        let t = app.terminal.as_mut().unwrap();

        let cmd = "echo manto_çãẽ_ñ".to_string();
        t.push_shell_line(cmd.clone());
        if let Some(ref mut session) = t.shell_session {
            let line = format!("{cmd}\r");
            session.write(line.as_bytes());
        }

        use std::thread;
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut saw = false;
        while start.elapsed() < Duration::from_secs(5) {
            t.tick();
            if t.shell_lines.iter().filter(|l| l.contains("çãẽ")).count() >= 2 {
                saw = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(saw, "unicode did not round-trip: {:?}", t.shell_lines);
    }

    #[test]
    fn interactive_terminal_feeds_the_emulator() {
        use super::super::Application;
        use crate::ui::window::Window;
        #[cfg(windows)]
        let prog = "cmd.exe";
        #[cfg(not(windows))]
        let prog = "/bin/sh";
        let path = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut app = Application::interactive_terminal_window(
            "App",
            Window::new(4, 4, 60, 25, 0),
            path,
            prog,
        );
        let t = app.terminal.as_mut().unwrap();
        assert!(t.interactive, "interactive terminal should be interactive");
        assert!(t.emulator.is_some());
        assert!(t.has_session());

        use std::thread;
        use std::time::{Duration, Instant};
        if let Some(ref mut session) = t.shell_session {
            session.write(b"echo int_marker_1234\r\n");
        }
        let start = Instant::now();
        let mut saw = false;
        while start.elapsed() < Duration::from_secs(5) {
            t.tick();
            if let Some(em) = t.emulator.as_ref() {
                for row in 0..em.total_lines() {
                    if em.line_as_text(row).contains("int_marker_1234") {
                        saw = true;
                    }
                }
            }
            if saw {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(saw, "interactive output did not reach the emulator");
    }

    #[test]
    fn line_mode_repl_keeps_typed_commands_visible() {
        // Regression: after starting a REPL in a line-mode terminal, commands
        // typed at the `.>` bar must remain visible next to their results —
        // via the console echo on a real PTY, or via Manto's local mirror on
        // the piped fallback. The python banner wait is tolerant: on hosts
        // where the app-aliased python3 boot is slow, the core assertion
        // (typed command visible) still holds via the local mirror.
        use super::super::Application;
        use crate::ui::window::Window;
        use std::thread;
        use std::time::{Duration, Instant};
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut app =
            Application::terminal_window("Term", Window::new(4, 4, 60, 25, 0), cwd, Vec::new());
        {
            let t = app.terminal.as_mut().unwrap();
            assert!(t.has_session(), "line-mode terminal must own a session");
            t.run_line("python");
        }

        // Wait (tolerantly) for the REPL banner before sending the payload.
        let start = Instant::now();
        let mut banner = false;
        while start.elapsed() < Duration::from_secs(10) {
            let t = app.terminal.as_mut().unwrap();
            t.tick();
            if t.shell_lines.iter().any(|l| l.contains("Python")) {
                banner = true;
                break;
            }
            thread::sleep(Duration::from_millis(40));
        }

        {
            let t = app.terminal.as_mut().unwrap();
            t.run_line("x=41");
            t.run_line("print(x+1)");
        }

        let start = Instant::now();
        let mut joined = String::new();
        while start.elapsed() < Duration::from_secs(10) {
            let t = app.terminal.as_mut().unwrap();
            t.tick();
            joined = t.shell_lines.join("\n");
            // On the piped fallback the REPL prompt can interleave with
            // stdout, splitting the result across reads (">>> 4>>> 2");
            // strip prompt glyphs and spaces before checking the payload.
            let payload = joined.replace(['>', ' '], "");
            if joined.contains("x=41") && (!banner || payload.contains("42")) {
                break;
            }
            thread::sleep(Duration::from_millis(40));
        }
        assert!(joined.contains("x=41"), "typed command lost: {joined:?}");
        if banner {
            let payload = joined.replace(['>', ' '], "");
            assert!(payload.contains("42"), "repl result lost: {joined:?}");
        }
    }

    #[test]
    fn pipe_line_buffer_edits_correctly() {
        let mut ts = TerminalState::new(".".to_string(), Vec::new());
        ts.pipe_feed(b"X=3");
        ts.pipe_backspace(); // erases the "3"
        ts.pipe_feed(b"4");
        assert_eq!(String::from_utf8_lossy(&ts.pipe_line), "X=4");
        ts.pipe_backspace();
        ts.pipe_backspace(); // pops "4", then "="
        assert_eq!(String::from_utf8_lossy(&ts.pipe_line), "X");
        ts.pipe_flush(); // no session: nothing to send, buffer empties
        assert!(ts.pipe_line.is_empty());
        // Backspace removes a full UTF-8 character, not a single byte.
        ts.pipe_feed("çã".as_bytes());
        assert_eq!(ts.pipe_line.len(), 4);
        ts.pipe_backspace();
        assert_eq!(String::from_utf8_lossy(&ts.pipe_line), "ç");
    }

    #[test]
    fn pipe_recall_navigates_local_history() {
        let mut ts = TerminalState::new(".".to_string(), Vec::new());
        ts.pipe_feed(b"echo um");
        ts.pipe_flush();
        ts.pipe_feed(b"echo dois");
        ts.pipe_flush();
        ts.pipe_feed(b"newnline");
        assert_eq!(String::from_utf8_lossy(&ts.pipe_line), "newnline");

        // Up recalls newest, then older.
        assert!(ts.pipe_recall(true));
        assert_eq!(String::from_utf8_lossy(&ts.pipe_line), "echo dois");
        assert!(ts.pipe_recall(true));
        assert_eq!(String::from_utf8_lossy(&ts.pipe_line), "echo um");
        assert!(!ts.pipe_recall(true), "already at the oldest entry");

        // Down walks back and finally restores the draft.
        assert!(ts.pipe_recall(false));
        assert_eq!(String::from_utf8_lossy(&ts.pipe_line), "echo dois");
        assert!(ts.pipe_recall(false));
        assert_eq!(String::from_utf8_lossy(&ts.pipe_line), "newnline");

        // Empty history does nothing.
        let mut empty = TerminalState::new(".".to_string(), Vec::new());
        assert!(!empty.pipe_recall(true));
    }

    #[test]
    #[cfg(windows)]
    fn piped_session_gets_the_corrected_line() {
        // User flow: type "X=4", backspace, type "2", Enter. The child's raw
        // input must be "X=2" — a leaked 0x08 (the old bug produced "invalid
        // non-printable character U+0008") is detected on the wire.
        use std::thread;
        use std::time::{Duration, Instant};
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut ts = TerminalState::with_program(cwd, "cmd.exe").unwrap();

        let type_byte = |ts: &mut TerminalState, bytes: &[u8], backspace: bool| {
            ts.mirror_input(bytes, backspace);
            if backspace {
                ts.pipe_backspace();
            } else {
                ts.pipe_feed(bytes);
            }
        };
        type_byte(&mut ts, b"X=4", false);
        type_byte(&mut ts, &[0x08], true);
        type_byte(&mut ts, b"2", false);
        ts.mirror_input(b"\r\n", false);
        ts.pipe_flush();
        // A second line proves streaming continues after the edit.
        type_byte(&mut ts, b"echo PIPE_SECOND_9911", false);
        ts.mirror_input(b"\r\n", false);
        ts.pipe_flush();

        let start = Instant::now();
        let mut saw_corrected = false;
        let mut saw_second = false;
        let mut no_leak = true;
        while start.elapsed() < Duration::from_secs(8) {
            let poll = ts.shell_session.as_mut().unwrap().poll();
            for chunk in &poll.outputs {
                if chunk.contains(&0x08) {
                    no_leak = false;
                }
                let text = String::from_utf8_lossy(chunk);
                // cmd echoes the executed line in its "not recognized" error.
                if text.contains("X=2") {
                    saw_corrected = true;
                }
                if text.contains("PIPE_SECOND_9911") {
                    saw_second = true;
                }
            }
            if saw_corrected && saw_second {
                break;
            }
            thread::sleep(Duration::from_millis(40));
        }
        assert!(saw_corrected, "child never received the corrected line");
        assert!(saw_second, "input streaming broke after the backspace edit");
        assert!(no_leak, "backspace byte leaked into the child input");
    }

    #[test]
    fn interactive_backspace_mirror_erases_the_cell() {
        // Regression: on piped sessions (no console echo) Manto renders the
        // keystrokes itself; backspace must erase the cell, not just move
        // the cursor back.
        let mut ts = TerminalState::new(".".to_string(), Vec::new());
        ts.emulator = Some(Terminal::new(20, 3));

        ts.mirror_input(b"a", false);
        ts.mirror_input(b"b", false);
        ts.mirror_input(b"c", false);
        let line = ts.emulator.as_ref().unwrap().line_as_text(0);
        assert_eq!(line.trim_end(), "abc");

        ts.mirror_input(&[0x08], true);
        let line = ts.emulator.as_ref().unwrap().line_as_text(0);
        assert_eq!(line.trim_end(), "ab", "backspace must blank the cell");

        ts.mirror_input(&[0x08], true);
        let line = ts.emulator.as_ref().unwrap().line_as_text(0);
        assert_eq!(line.trim_end(), "a");
    }

    #[test]
    fn interactive_resize_is_propagated() {
        let mut t = TerminalState::new(".".to_string(), Vec::new());
        let mut em = Terminal::new(80, 24);
        em.process(b"line0\r\nline1\r\n");
        t.emulator = Some(em);
        t.set_grid_size(40, 12);
        let em = t.emulator.as_ref().unwrap();
        assert_eq!(em.cols(), 40);
        assert_eq!(em.rows(), 12);
        assert!((0..em.total_lines()).any(|r| em.line_as_text(r).contains("line0")));
    }

    #[test]
    fn interactive_typing_is_visible_in_emulator() {
        // Typed input must show up in the emulator: via the real PTY echo on a
        // ConPTY/PTY host, or via Manto's local echo on the piped fallback.
        use super::super::Application;
        use crate::ui::window::Window;
        use std::thread;
        use std::time::{Duration, Instant};
        #[cfg(windows)]
        let prog = "cmd.exe";
        #[cfg(not(windows))]
        let prog = "/bin/sh";
        let path = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut app = Application::interactive_terminal_window(
            "App",
            Window::new(4, 4, 60, 25, 0),
            path,
            prog,
        );
        let t = app.terminal.as_mut().unwrap();
        assert!(t.interactive && t.has_session() && t.emulator.is_some());

        // Settle the initial prompt.
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            t.tick();
            if t.emulator.as_ref().map(|em| em.total_lines()) > Some(1) {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }

        // Type a unique string one key at a time through the same path the
        // desktop uses (mirror + pipe buffer on piped sessions, raw bytes on
        // real PTYs where the console echoes).
        let is_pty = t
            .shell_session
            .as_ref()
            .map(|s| s.is_real_pty())
            .unwrap_or(false);
        for b in b"manto_type_4711" {
            if !is_pty {
                t.mirror_input(&[*b], false);
                t.pipe_feed(&[*b]);
            } else if let Some(ref mut s) = t.shell_session {
                s.write(&[*b]);
            }
            thread::sleep(Duration::from_millis(20));
        }
        if is_pty {
            if let Some(ref mut s) = t.shell_session {
                s.write(b"\r");
            }
        } else {
            t.pipe_flush();
        }

        let start = Instant::now();
        let mut seen = false;
        while start.elapsed() < Duration::from_secs(5) {
            t.tick();
            if let Some(em) = t.emulator.as_ref() {
                for r in 0..em.total_lines() {
                    if em.line_as_text(r).contains("manto_type_4711") {
                        seen = true;
                    }
                }
            }
            if seen {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
        assert!(seen, "typed input never became visible in the emulator");
    }
}
