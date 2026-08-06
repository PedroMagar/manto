use crate::cmd::{CommandEntry, tick_all};
use crate::terminal_backend::CommandSession;

/// Rewrite known REPL commands to their explicit interactive form.
///
/// In the pipe fallback (host without a real pseudo-terminal), `python`
/// without `-i` reads stdin as a script and only executes at EOF; with `-i`
/// the REPL processes line by line. Only bare invocations (no arguments)
/// are rewritten.
pub fn interactive_command(cmd: &str) -> String {
    match cmd.trim() {
        // Under a pipe (no real PTY) `python` reads stdin as a script until
        // EOF; with `-i` the REPL processes line by line and shows the prompt.
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
    /// Prompt of the running REPL/interactive application (e.g. Python's
    /// ">>>"). When Some, the window hides the " .> " bar and uses this
    /// prompt instead.
    pub repl_prompt:  Option<String>,
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
        // Spawn a shell that will persist for the lifetime of this terminal
        // window, running as an interactive session.
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

    /// Advance one tick: drain session output and tick the commands.
    /// Returns true if anything changed.
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
        // exit/quit leave the REPL; ordinary commands do not.
        for e in ["exit", "exit()", "quit", "quit()", "\\q", ":q"] {
            assert!(is_repl_exit(e), "{e} should exit the REPL");
        }
        for e in ["dir", "print('x')", "q", "1+1"] {
            assert!(!is_repl_exit(e), "{e} should not exit the REPL");
        }
    }

    #[test]
    fn repl_prompt_is_suppressed_and_used_as_prefix() {
        let mut t = TerminalState::new(".".to_string(), Vec::new());

        // A bare ">>>" line does not go to the display; it becomes the prefix.
        t.ingest_output_line(">>>".to_string());
        assert_eq!(t.repl_prompt.as_deref(), Some(">>>"));
        assert!(t.shell_lines.is_empty(), "prompt leaked: {:?}", t.shell_lines);

        // REPL results flow normally and keep the mode.
        t.ingest_output_line("42".to_string());
        assert!(t.shell_lines.iter().any(|l| l == "42"));
        assert_eq!(t.repl_prompt.as_deref(), Some(">>>"));

        // clear_repl leaves the mode.
        t.clear_repl();
        assert_eq!(t.repl_prompt, None);
        assert!(t.repl_prompt.is_none());
    }

    #[test]
    fn terminal_session_echo_and_output_accumulate() {
        // Terminal window with a real session. On hosts without ConPTY the
        // pipe fallback is used; the flow (local echo + shell output) must
        // accumulate in shell_lines.
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

        // Type a command (local echo).
        t.cmd_input = "echo echo_marker_9911".to_string();
        t.input_cursor = t.cmd_input.chars().count();

        // Enter: local echo + send to the shell.
        let cmd = t.cmd_input.trim().to_string();
        t.push_shell_line(cmd.clone());
        if let Some(ref mut session) = t.shell_session {
            let line = format!("{}\r", cmd);
            session.write(line.as_bytes());
        }
        t.cmd_input.clear();
        t.input_cursor = 0;

        // The local echo appears immediately.
        assert!(t.shell_lines.iter().any(|l| l.contains("echo_marker_9911")));

        // The shell output arrives via poll.
        use std::thread;
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut saw_output = false;
        while start.elapsed() < Duration::from_secs(5) {
            t.tick();
            // The shell repeats the line and/or emits the result.
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
        // Simulates recording commands executed in the session (done on
        // Enter) and checks that Up/Down navigate the local history.
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

        // Two executed commands.
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

        // Open python (rewritten to `python -i` on Enter).
        let rev = interactive_command("python");
        let line = format!("{}\r\n", rev);
        if let Some(ref mut session) = t.shell_session {
            session.write(line.as_bytes());
        }

        // Python must (a) open (banner) and (b) keep accepting lines.
        let start = Instant::now();
        let mut saw_banner = false;
        while start.elapsed() < Duration::from_secs(6) {
            if let Some(session) = t.shell_session.as_mut() {
                let poll = session.poll();
                for l in poll.lines {
                    if l.contains("Python") && l.contains("on win32") {
                        saw_banner = true;
                    }
                }
            }
            if saw_banner { break; }
            thread::sleep(Duration::from_millis(40));
        }
        assert!(saw_banner, "python did not open (no banner)");

        // The input must reach python (\r\n line ending).
        if let Some(ref mut session) = t.shell_session {
            let line = "print('PY_APP_MARK_69')\r\n".to_string();
            session.write(line.as_bytes());
        }
        let start = Instant::now();
        let mut saw_mark = false;
        while start.elapsed() < Duration::from_secs(5) {
            if let Some(session) = t.shell_session.as_mut() {
                let poll = session.poll();
                for l in poll.lines {
                    if l.contains("PY_APP_MARK_69") {
                        saw_mark = true;
                    }
                }
            }
            if saw_mark { break; }
            thread::sleep(Duration::from_millis(40));
        }
        assert!(saw_mark, "python did not execute the sent line");
    }

    #[test]
    fn terminal_window_with_history_preserves_it() {
        // Ctrl+Enter: the detached terminal must preserve the dock history.
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
}
