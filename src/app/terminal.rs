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
        c if c.eq_ignore_ascii_case("python")  => "python -i".to_string(),
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
        "python" | "python3" | "python2" | "python3.11" | "python3.12"
            | "ipython" | "vscode" | "vim" | "vim.tiny" | "nano" | "emacs" | "pico"
            | "less" | "more" | "top" | "htop" | "btop" | "lazygit" | "fzf"
            | "node" | "node.exe" | "bash" | "sh" | "zsh" | "cmd" | "cmd.exe"
            | "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
    )
}

/// Strip a leading `#i` directive: `#i vim` runs `vim` interactively, a bare
/// `#i` opens the default shell. Returns (command, interactive).
pub fn split_interactive_flag(raw: &str) -> (String, bool) {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed[..2].eq_ignore_ascii_case("#i") {
        if let Some(rest) = trimmed.get(2..) {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return (rest.trim_start().to_string(), true);
            }
        }
    }
    (trimmed.to_string(), false)
}

/// Default interactive shell for the platform.
pub fn default_shell() -> String {
    #[cfg(not(windows))]
    {
        "/bin/sh".to_string()
    }
    #[cfg(windows)]
    {
        if std::path::Path::new("pwsh.exe").exists() {
            "pwsh.exe".to_string()
        } else if std::path::Path::new("powershell.exe").exists() {
            "powershell.exe".to_string()
        } else {
            std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
        }
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
        Key::Backspace => buf.push(0x7f),
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

pub struct TerminalState {
    /// Persistent shell session for this terminal window.
    /// When Some, raw keyboard input is forwarded to the shell.
    pub shell_session: Option<CommandSession>,
    /// Accumulated raw output lines drained from the shell session
    /// (classic line-mode viewer).
    pub shell_lines:   Vec<String>,
    /// Terminal emulator (grid) fed from the raw bytes. Present when the
    /// window is an interactive terminal.
    pub emulator:   Option<Terminal>,
    /// Interactive passthrough: when true, focus-mode keys forward raw to the
    /// session and the window renders the emulator grid.
    pub interactive: bool,
    /// Historical command entries (displayed in the terminal content).
    pub commands:     Vec<CommandEntry>,
    pub cmd_input:    String,
    pub input_cursor: usize,
    pub panel_scroll: usize,
    pub path:         String,
    pub history_index: Option<usize>,
    pub history_draft: Option<String>,
    /// Prompt of the running REPL/interactive application (e.g. Python's
    /// ">>>"). When Some, the window hides the " .> " bar and uses this
    /// prompt instead.
    pub repl_prompt:  Option<String>,
    /// Partial line buffer (bytes not yet terminated by '\n').
    tail: String,
}

impl TerminalState {
    /// Detect a REPL prompt in the output stream (e.g. Python's ">>>",
    /// "sqlite>") so it is not displayed as a loose line.
    fn looks_like_repl_prompt(line: &str) -> bool {
        let t = line.trim();
        !t.is_empty() && t.chars().count() <= 12 && t.ends_with('>')
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
            tail: String::new(),
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
            tail: String::new(),
        })
    }

    /// Interactive terminal: run `program` directly (emulator renders output,
    /// keys forwarded raw).
    pub fn with_program(path: String, program: &str) -> Result<Self, String> {
        let mut session = CommandSession::spawn(program, &path)?;
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
            tail: String::new(),
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

    /// Advance one tick: drain session output, feed the emulator (interactive)
    /// and reconstruct display lines for the line-mode viewer.
    /// Returns true if anything changed.
    pub fn tick(&mut self) -> bool {
        let mut changed = tick_all(&mut self.commands);
        if let Some(ref mut session) = self.shell_session {
            let poll = session.poll();
            let got_bytes = !poll.outputs.is_empty();
            for chunk in &poll.outputs {
                if let Some(em) = self.emulator.as_mut() {
                    em.process(chunk);
                }
                self.tail.push_str(&String::from_utf8_lossy(chunk));
            }
            // Emulator-only progress (no newline) still needs a redraw.
            if got_bytes {
                changed = true;
            }
            // Split complete lines.
            loop {
                match self.tail.find('\n') {
                    Some(pos) => {
                        let line: String = self.tail.drain(..=pos).collect();
                        let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
                        if !trimmed.is_empty() {
                            changed |= self.ingest_output_line(trimmed);
                        }
                    }
                    None => break,
                }
            }
            if poll.closed {
                self.repl_prompt = None;
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
    pub fn has_session(&self) -> bool {
        self.shell_session.is_some()
    }

    /// Resize the emulator grid and propagate to the child process.
    pub fn set_grid_size(&mut self, cols: u16, rows: u16) {
        if let Some(em) = self.emulator.as_mut() {
            if em.cols() != cols || em.rows() != rows {
                em.resize(cols, rows);
            }
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
        assert_eq!(split_interactive_flag("#i   python3"), ("python3".to_string(), true));
        assert_eq!(split_interactive_flag("#I top"), ("top".to_string(), true));
        // Bare `#i` opens the default shell.
        assert_eq!(split_interactive_flag("#i"), ("".to_string(), true));
        assert_eq!(split_interactive_flag("#i   "), ("".to_string(), true));
        // Not a directive: no marker, or `#i` glued to a word.
        assert_eq!(split_interactive_flag("vim #i"), ("vim #i".to_string(), false));
        assert_eq!(split_interactive_flag("#iffy"), ("#iffy".to_string(), false));
        assert_eq!(split_interactive_flag("# comment"), ("# comment".to_string(), false));
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
        assert_eq!(key_to_bytes(Key::PageUp).unwrap(), b"\x1b[5~");
        assert_eq!(key_to_bytes(Key::Delete).unwrap(), b"\x1b[3~");
        assert_eq!(key_to_bytes(Key::CtrlC).unwrap(), b"\x03");
        assert_eq!(key_to_bytes(Key::CtrlZ).unwrap(), b"\x1a");
        assert_eq!(key_to_bytes(Key::Char('a')).unwrap(), b"a");
        // Desktop shortcuts carry no terminal meaning.
        assert!(key_to_bytes(Key::Ctrl1).is_none());
        assert!(key_to_bytes(Key::AltR).is_none());
    }

    #[test]
    fn repl_prompt_is_suppressed_and_used_as_prefix() {
        let mut t = TerminalState::new(".".to_string(), Vec::new());

        t.ingest_output_line(">>>".to_string());
        assert_eq!(t.repl_prompt.as_deref(), Some(">>>"));
        assert!(t.shell_lines.is_empty(), "prompt leaked: {:?}", t.shell_lines);

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
            let line = format!("{}\r", cmd);
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
            if t.shell_lines.iter().filter(|l| l.contains("echo_marker_9911")).count() >= 2 {
                saw_output = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(saw_output, "shell did not emit result lines: {:?}", t.shell_lines);
    }

    #[test]
    fn terminal_session_history_supports_navigation() {
        use super::super::Application;
        use crate::cmd::CommandEntry;
        use crate::input;
        use crate::ui::window::Window;
        let mut app = Application::terminal_window(
            "Term",
            Window::new(4, 4, 60, 25, 0),
            ".".to_string(),
            Vec::new(),
        );
        let t = app.terminal.as_mut().unwrap();

        for cmd in ["echo first_cmd", "echo second_cmd"] {
            t.commands.push(CommandEntry::completed(cmd, &t.path, Vec::new()));
        }

        let mut input = String::new();
        let mut index = None;
        let mut draft = None;

        assert!(input::history_up(&t.commands, &mut input, &mut index, &mut draft));
        assert_eq!(input, "echo second_cmd");
        assert!(input::history_up(&t.commands, &mut input, &mut index, &mut draft));
        assert_eq!(input, "echo first_cmd");
        assert!(input::history_down(&t.commands, &mut input, &mut index, &mut draft));
        assert_eq!(input, "echo second_cmd");
    }

    #[test]
    fn python_opens_through_session() {
        use super::super::Application;
        use crate::ui::window::Window;
        use std::thread;
        use std::time::{Duration, Instant};
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
        let mut app = Application::terminal_window("Term", Window::new(4, 4, 60, 25, 0), cwd.clone(), Vec::new());
        let t = app.terminal.as_mut().unwrap();
        assert!(t.has_session());

        let rev = interactive_command("python");
        let line = format!("{}\r\n", rev);
        if let Some(ref mut session) = t.shell_session {
            session.write(line.as_bytes());
        }

        let start = Instant::now();
        let mut saw_banner = false;
        while start.elapsed() < Duration::from_secs(6) {
            t.tick();
            if t.shell_lines.iter().any(|l| l.contains("Python") && l.contains("on win32")) {
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
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
        let commands = vec![
            CommandEntry::completed("cd xphmg", &cwd, vec!["erro: diretório não existe".to_string()]),
            CommandEntry::completed("flutter --version", &cwd, vec!["Flutter 3.32.8 stable".to_string()]),
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
            let line = format!("{}\r", cmd);
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
        let path = std::env::current_dir().unwrap().to_string_lossy().to_string();
        let mut app = Application::interactive_terminal_window("App", Window::new(4, 4, 60, 25, 0), path, prog);
        let t = app.terminal.as_mut().unwrap();
        assert!(t.interactive, "interactive terminal should be interactive");
        assert!(t.emulator.is_some());
        assert!(t.has_session());

        use std::thread;
        use std::time::{Duration, Instant};
        if let Some(ref mut session) = t.shell_session {
            let _ = session.write(b"echo int_marker_1234\r\n");
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
            if saw { break; }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(saw, "interactive output did not reach the emulator");
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
        let path = std::env::current_dir().unwrap().to_string_lossy().to_string();
        let mut app = Application::interactive_terminal_window("App", Window::new(4, 4, 60, 25, 0), path, prog);
        let t = app.terminal.as_mut().unwrap();
        assert!(t.interactive && t.has_session() && t.emulator.is_some());

        // Settle the initial prompt.
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            t.tick();
            if t.emulator.as_ref().map(|em| em.total_lines()) > Some(1) { break; }
            thread::sleep(Duration::from_millis(30));
        }

        // Type a unique string one key at a time (no Enter yet).
        if let Some(ref mut s) = t.shell_session {
            for b in b"manto_type_4711\x0d" {
                let _ = s.write(&[*b]);
                thread::sleep(Duration::from_millis(20));
            }
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
            if seen { break; }
            thread::sleep(Duration::from_millis(30));
        }
        assert!(seen, "typed input never became visible in the emulator");
    }
}
