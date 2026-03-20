use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use super::TerminalUpdate;

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

pub fn spawn(command: &str, cwd: &str, tx: Sender<TerminalUpdate>) -> Result<PlatformCommand, String> {
    let mut child = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn shell: {err}"))?;

    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx.send(TerminalUpdate::Line(line));
            }
            let _ = tx.send(TerminalUpdate::Closed);
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx.send(TerminalUpdate::Line(line));
            }
            let _ = tx.send(TerminalUpdate::Closed);
        });
    }

    Ok(PlatformCommand { child })
}
