use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::thread;

use super::TerminalUpdate;

fn decode_output(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }

    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }

    let looks_like_utf16le = bytes.len() >= 4
        && bytes.chunks(2).filter(|chunk| chunk.len() == 2 && chunk[1] == 0).count() * 2 >= bytes.len() / 3;
    if looks_like_utf16le {
        let units: Vec<u16> = bytes.chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }

    String::from_utf8_lossy(bytes).into_owned()
}

fn emit_output(bytes: Vec<u8>, tx: std::sync::mpsc::Sender<TerminalUpdate>) {
    let text = decode_output(&bytes);
    for line in text.lines() {
        let trimmed = line.trim_end_matches('\r').to_string();
        let _ = tx.send(TerminalUpdate::Line(trimmed));
    }
    let _ = tx.send(TerminalUpdate::Closed);
}

pub struct PlatformCommand {
    child: Child,
}

impl PlatformCommand {
    pub fn try_wait(&mut self) -> Option<i32> {
        self.child.try_wait().ok().flatten().map(|status| status.code().unwrap_or_default())
    }
}

impl Drop for PlatformCommand {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_with_program(
    program: &str,
    args: &[&str],
    command: &str,
    cwd: &str,
) -> Result<Child, String> {
    let mut cmd = Command::new(program);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if !cwd.trim().is_empty() {
        cmd.current_dir(cwd);
    }

    cmd.spawn().map_err(|err| format!("failed to spawn shell: {err}"))
}

pub fn spawn(command: &str, cwd: &str, tx: std::sync::mpsc::Sender<TerminalUpdate>) -> Result<PlatformCommand, String> {
    let mut child = spawn_with_program("powershell.exe", &["-NoProfile", "-Command"], command, cwd)
        .or_else(|_| spawn_with_program("pwsh.exe", &["-NoProfile", "-Command"], command, cwd))
        .or_else(|_| {
            let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
            spawn_with_program(&shell, &["/D", "/C"], command, cwd)
        })?;

    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut stdout = stdout;
            let mut bytes = Vec::new();
            if stdout.read_to_end(&mut bytes).is_ok() {
                emit_output(bytes, tx);
            } else {
                let _ = tx.send(TerminalUpdate::Closed);
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut stderr = stderr;
            let mut bytes = Vec::new();
            if stderr.read_to_end(&mut bytes).is_ok() {
                emit_output(bytes, tx);
            } else {
                let _ = tx.send(TerminalUpdate::Closed);
            }
        });
    }

    Ok(PlatformCommand { child })
}
