use std::collections::HashSet;
use std::path::{Path, PathBuf};
use crate::cmd::CommandEntry;

pub fn history_up(commands: &[CommandEntry], input: &mut String, index: &mut Option<usize>, draft: &mut Option<String>) -> bool {
    if commands.is_empty() {
        return false;
    }

    let next = match *index {
        Some(current) if current > 0 => current - 1,
        Some(_) => 0,
        None => {
            *draft = Some(input.clone());
            commands.len() - 1
        }
    };

    *index = Some(next);
    *input = commands[next].command.clone();
    true
}

pub fn history_down(commands: &[CommandEntry], input: &mut String, index: &mut Option<usize>, draft: &mut Option<String>) -> bool {
    let Some(current) = *index else {
        return false;
    };

    if current + 1 < commands.len() {
        let next = current + 1;
        *index = Some(next);
        *input = commands[next].command.clone();
    } else {
        *index = None;
        *input = draft.take().unwrap_or_default();
    }
    true
}

pub fn reset_history_navigation(index: &mut Option<usize>, draft: &mut Option<String>) {
    *index = None;
    *draft = None;
}

pub fn input_char_len(input: &str) -> usize {
    input.chars().count()
}

pub fn cursor_to_byte(input: &str, cursor: usize) -> usize {
    input.char_indices().nth(cursor).map(|(idx, _)| idx).unwrap_or(input.len())
}

pub fn move_input_cursor_left(cursor: &mut usize) -> bool {
    if *cursor == 0 {
        false
    } else {
        *cursor -= 1;
        true
    }
}

pub fn move_input_cursor_right(input: &str, cursor: &mut usize) -> bool {
    let len = input_char_len(input);
    if *cursor >= len {
        false
    } else {
        *cursor += 1;
        true
    }
}

pub fn insert_input_char(input: &mut String, cursor: &mut usize, ch: char) {
    let byte = cursor_to_byte(input, *cursor);
    input.insert(byte, ch);
    *cursor += 1;
}

pub fn backspace_input_char(input: &mut String, cursor: &mut usize) -> bool {
    if *cursor == 0 {
        return false;
    }

    let end = cursor_to_byte(input, *cursor);
    let start = cursor_to_byte(input, *cursor - 1);
    input.replace_range(start..end, "");
    *cursor -= 1;
    true
}

pub fn delete_input_char(input: &mut String, cursor: &mut usize) -> bool {
    let len = input_char_len(input);
    if *cursor >= len {
        return false;
    }

    let start = cursor_to_byte(input, *cursor);
    let end = cursor_to_byte(input, *cursor + 1);
    input.replace_range(start..end, "");
    true
}

pub fn input_view(input: &str, cursor: usize, max_len: usize) -> (String, usize) {
    if max_len == 0 {
        return (String::new(), 0);
    }

    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    if len <= max_len {
        return (input.to_string(), cursor.min(len));
    }

    let mut start = cursor.saturating_sub(max_len.saturating_sub(1));
    if start + max_len > len {
        start = len.saturating_sub(max_len);
    }

    let end = (start + max_len).min(len);
    let display: String = chars[start..end].iter().collect();
    (display, cursor.saturating_sub(start).min(max_len))
}

pub fn token_bounds(input: &str, cursor: usize) -> (usize, usize) {
    let chars: Vec<char> = input.chars().collect();
    let cursor = cursor.min(chars.len());

    let mut start = cursor;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }

    let mut end = cursor;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }

    (start, end)
}

pub fn replace_token(input: &mut String, cursor: &mut usize, start: usize, end: usize, replacement: &str) {
    let start_byte = cursor_to_byte(input, start);
    let end_byte = cursor_to_byte(input, end);
    input.replace_range(start_byte..end_byte, replacement);
    *cursor = start + replacement.chars().count();
}

pub fn longest_common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };

    let mut prefix: Vec<char> = first.chars().collect();
    for value in &values[1..] {
        let chars: Vec<char> = value.chars().collect();
        let common = prefix.iter().zip(chars.iter()).take_while(|(a, b)| a == b).count();
        prefix.truncate(common);
        if prefix.is_empty() {
            break;
        }
    }

    prefix.into_iter().collect()
}

fn path_token_parts(token: &str) -> (String, String) {
    match token.rfind(['\\', '/']) {
        Some(idx) => {
            let split = idx + 1;
            (token[..split].to_string(), token[split..].to_string())
        }
        None => (String::new(), token.to_string()),
    }
}

pub fn collect_path_candidates(current_path: &str, token: &str, dirs_only: bool) -> Vec<(String, bool)> {
    let (base_part, leaf) = path_token_parts(token);
    let base_path = if base_part.is_empty() {
        PathBuf::from(current_path)
    } else {
        let base = PathBuf::from(&base_part);
        if base.is_absolute() {
            base
        } else {
            Path::new(current_path).join(&base_part)
        }
    };

    let mut candidates = Vec::new();
    let Ok(entries) = std::fs::read_dir(&base_path) else {
        return candidates;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if dirs_only && !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if !name.to_ascii_lowercase().starts_with(&leaf.to_ascii_lowercase()) {
            continue;
        }

        let mut display = format!("{}{}", base_part, name);
        if file_type.is_dir() {
            display.push(std::path::MAIN_SEPARATOR);
        }
        candidates.push((display, file_type.is_dir()));
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates
}

pub fn collect_command_candidates(current_path: &str, prefix: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    let prefix_lower = prefix.to_ascii_lowercase();

    let mut search_dirs = vec![PathBuf::from(current_path)];
    if let Some(path_var) = std::env::var_os("PATH") {
        search_dirs.extend(std::env::split_paths(&path_var));
    }

    #[cfg(windows)]
    let pathext: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(|ext| ext.to_ascii_lowercase())
        .collect();

    for dir in search_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                continue;
            }

            let file_name = entry.file_name().to_string_lossy().to_string();

            #[cfg(windows)]
            let candidate = {
                let path = entry.path();
                let ext = path.extension()
                    .map(|ext| format!(".{}", ext.to_string_lossy().to_ascii_lowercase()))
                    .unwrap_or_default();
                if !pathext.iter().any(|allowed| allowed == &ext) {
                    continue;
                }
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or(file_name.clone())
            };

            #[cfg(unix)]
            let candidate = {
                use std::os::unix::fs::PermissionsExt;
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                if meta.permissions().mode() & 0o111 == 0 {
                    continue;
                }
                file_name.clone()
            };

            if !candidate.to_ascii_lowercase().starts_with(&prefix_lower) {
                continue;
            }

            let key = candidate.to_ascii_lowercase();
            if seen.insert(key) {
                candidates.push(candidate);
            }
        }
    }

    candidates.sort();
    candidates
}

pub fn autocomplete_input(input: &mut String, cursor: &mut usize, current_path: &str) -> bool {
    let (start, end) = token_bounds(input, *cursor);
    let chars: Vec<char> = input.chars().collect();
    let token: String = chars[start..end].iter().collect();
    let first_token_end = chars.iter().position(|c| c.is_whitespace()).unwrap_or(chars.len());
    let first_token: String = chars[..first_token_end].iter().collect();

    let suggestions: Vec<String> = if start == 0 {
        collect_command_candidates(current_path, &token)
    } else if first_token == "cd" {
        collect_path_candidates(current_path, &token, true).into_iter().map(|(text, _)| text).collect()
    } else if token.contains(['\\', '/']) || token.starts_with('.') {
        collect_path_candidates(current_path, &token, false).into_iter().map(|(text, _)| text).collect()
    } else {
        Vec::new()
    };

    if suggestions.is_empty() {
        return false;
    }

    let replacement = if suggestions.len() == 1 {
        let mut value = suggestions[0].clone();
        if start == 0 && !value.ends_with(' ') {
            value.push(' ');
        }
        value
    } else {
        let lcp = longest_common_prefix(&suggestions);
        if lcp.chars().count() <= token.chars().count() {
            return false;
        }
        lcp
    };

    replace_token(input, cursor, start, end, &replacement);
    true
}
