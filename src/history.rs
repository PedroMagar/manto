use std::fs::{OpenOptions, File};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

pub struct History {
    path: PathBuf,
}

impl History {
    pub fn new() -> Self {
        let path = match (std::env::var_os("XDG_DATA_HOME"), cfg!(windows)) {
            (_, true) => {
                if let Some(appdata) = std::env::var_os("APPDATA") {
                    PathBuf::from(appdata).join("manto").join("history")
                } else {
                    home_dir().unwrap_or_else(|| PathBuf::from("."))
                        .join(".manto").join("history")
                }
            }
            (Some(xdg), false) => PathBuf::from(xdg).join("manto").join("history"),
            (None, false) => home_dir().unwrap_or_else(|| PathBuf::from("."))
                .join(".manto").join("history"),
        };

        Self { path }
    }

    /// Load up to `max_lines` lines from the history file.
    pub fn load(&self, max_lines: usize) -> Vec<String> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };

        let reader = io::BufReader::new(file);
        let total_lines: Vec<String> = reader.lines()
            .filter_map(|l| l.ok())
            .collect();

        let start = total_lines.len().saturating_sub(max_lines);
        total_lines.into_iter().skip(start).collect()
    }

    /// Append a single line to the history file.
    pub fn append(&self, line: &str) {
        if line.trim().is_empty() {
            return;
        }

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{}", line);
        }
    }

    /// Get the path to the history file (for testing/debugging).
    #[cfg(test)]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_empty_returns_none() {
        let history = History::new();
        let loaded = history.load(100);
        assert!(loaded.is_empty());
    }

    #[test]
    fn load_returns_existing_history() {
        let tmp_dir = std::env::temp_dir().join(format!("manto-history-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        let _ = fs::create_dir_all(&tmp_dir);

        let history_file = tmp_dir.join("history");
        {
            let mut file = File::create(&history_file).unwrap();
            writeln!(file, "echo hello").unwrap();
            writeln!(file, "ls -la").unwrap();
            writeln!(file, "pwd").unwrap();
        }

        // Override the path temporarily for testing
        let history = History { path: history_file };
        let loaded = history.load(100);

        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0], "echo hello");
        assert_eq!(loaded[1], "ls -la");
        assert_eq!(loaded[2], "pwd");

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn append_creates_file_and_line() {
        let tmp_dir = std::env::temp_dir().join(format!("manto-append-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        let _ = fs::create_dir_all(&tmp_dir);

        let history_file = tmp_dir.join("history");
        let history = History { path: history_file.clone() };
        history.append("test command");

        let loaded = fs::read_to_string(&history_file).unwrap();
        assert!(loaded.contains("test command"));

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn append_ignores_empty_lines() {
        let tmp_dir = std::env::temp_dir().join(format!("manto-empty-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        let _ = fs::create_dir_all(&tmp_dir);

        let history_file = tmp_dir.join("history");
        let history = History { path: history_file.clone() };
        history.append("   ");
        history.append("");

        let content = fs::read_to_string(&history_file).unwrap_or_default();
        assert!(content.trim().is_empty());

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
