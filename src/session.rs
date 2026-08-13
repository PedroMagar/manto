// Session persistence: window layout and active desktop between runs.
//
// On quit Manto saves the geometry (position/size), title, desktop and working
// directory of every open terminal window to `~/.manto/session.json`; on the
// next start those windows are re-created as fresh terminals with the saved
// geometry. Shell sessions themselves are host processes and do not survive,
// but the desktop layout does.
//
// JSON is produced by hand (small, well-defined) and read by `crate::json`.

use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SavedApp {
    pub title: String,
    pub path: String,
    pub desktop: usize,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

#[derive(Debug, Default)]
pub struct Session {
    pub current_desktop: usize,
    pub apps: Vec<SavedApp>,
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

fn session_path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".manto")
        .join("session.json")
}

fn json_str(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn render_session(session: &Session) -> String {
    let mut out = String::from("{\n  \"current_desktop\": ");
    out.push_str(&session.current_desktop.to_string());
    out.push_str(",\n  \"apps\": [\n");
    for (i, app) in session.apps.iter().enumerate() {
        out.push_str("    {");
        out.push_str(&format!("\"title\": {},", json_str(&app.title)));
        out.push_str(&format!("\"path\": {},", json_str(&app.path)));
        out.push_str(&format!("\"desktop\": {},", app.desktop));
        out.push_str(&format!("\"x\": {},", app.x));
        out.push_str(&format!("\"y\": {},", app.y));
        out.push_str(&format!("\"w\": {},", app.w));
        out.push_str(&format!("\"h\": {}", app.h));
        out.push('}');
        if i + 1 < session.apps.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

/// Persist the session to `~/.manto/session.json`. Returns true on success.
pub fn save(session: &Session) -> bool {
    save_to(session, &session_path())
}

/// Persist the session to `path`. Returns true on success.
pub fn save_to(session: &Session, path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if !parent.is_dir() && std::fs::create_dir_all(parent).is_err() {
        return false;
    }

    let out = render_session(session);
    match std::fs::File::create(path) {
        Ok(mut file) => {
            let _ = file.write_all(out.as_bytes());
            let _ = file.flush();
            true
        }
        Err(_) => false,
    }
}

/// Load a previously saved session, if any. Invalid files yield None.
pub fn load() -> Option<Session> {
    load_from(&session_path())
}

/// Load a session from `path`. Invalid files yield None.
pub fn load_from(path: &Path) -> Option<Session> {
    let source = std::fs::read_to_string(path).ok()?;
    let json = crate::json::parse(&source).ok()?;

    let current_desktop = json
        .field("current_desktop")
        .and_then(|v| v.as_f64())
        .map(|n| (n as i64).clamp(1, crate::ui::DESKTOP_COUNT as i64) as usize)
        .unwrap_or(1);

    let apps = json
        .field("apps")
        .and_then(|v| v.as_arr())
        .map(|items| {
            items
                .iter()
                .filter_map(saved_app_from_json)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(Session {
        current_desktop,
        apps,
    })
}

fn saved_app_from_json(json: &crate::json::Json) -> Option<SavedApp> {
    let u16_field = |key: &str| {
        json.field(key)
            .and_then(|v| v.as_f64())
            .map(|n| n.max(0.0) as u16)
    };
    Some(SavedApp {
        title: json
            .field("title")
            .and_then(|v| v.str_value())
            .map(str::to_owned)?,
        path: json
            .field("path")
            .and_then(|v| v.str_value())
            .unwrap_or_default()
            .to_string(),
        desktop: json
            .field("desktop")
            .and_then(|v| v.as_f64())
            .map(|n| (n as i64).clamp(1, 4) as usize)
            .unwrap_or(1),
        x: u16_field("x").unwrap_or(0),
        y: u16_field("y").unwrap_or(0),
        w: u16_field("w").unwrap_or(1),
        h: u16_field("h").unwrap_or(1),
    })
}

/// True when `path` still exists (i.e. the saved working directory is usable).
pub fn path_exists(path: &str) -> bool {
    Path::new(path).is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_session_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("manto-{name}-{}", std::process::id()))
    }

    fn sample_session() -> Session {
        Session {
            current_desktop: 3,
            apps: vec![
                SavedApp {
                    title: "Terminal 1".to_string(),
                    path: r"D:\projetos\manto".to_string(),
                    desktop: 2,
                    x: 10,
                    y: 4,
                    w: 60,
                    h: 20,
                },
                SavedApp {
                    title: "App \"x\"".to_string(),
                    path: "~".to_string(),
                    desktop: 3,
                    x: 30,
                    y: 10,
                    w: 40,
                    h: 15,
                },
            ],
        }
    }

    #[test]
    fn session_roundtrips_through_disk() {
        let path = temp_session_file("session-roundtrip");
        let session = sample_session();

        assert!(save_to(&session, &path), "save must succeed");

        let loaded = load_from(&path).expect("saved session must load");
        assert_eq!(loaded.current_desktop, 3);
        assert_eq!(loaded.apps.len(), 2);
        assert_eq!(loaded.apps[0].title, "Terminal 1");
        assert_eq!(loaded.apps[0].path, r"D:\projetos\manto");
        assert_eq!(loaded.apps[0].desktop, 2);
        assert_eq!(
            (
                loaded.apps[0].x,
                loaded.apps[0].y,
                loaded.apps[0].w,
                loaded.apps[0].h
            ),
            (10, 4, 60, 20)
        );
        assert_eq!(loaded.apps[1].title, "App \"x\"");
        assert_eq!(loaded.apps[1].desktop, 3);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn invalid_file_loads_as_none() {
        let path = temp_session_file("session-invalid");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_from(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_loads_as_none() {
        let path = temp_session_file("session-missing");
        let _ = std::fs::remove_file(&path);
        assert!(load_from(&path).is_none());
    }
}
