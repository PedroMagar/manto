use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use super::TerminalUpdate;

// ── Windows ConPTY FFI types ──────────────────────────────────────────────────

type Handle = *mut u8;
type Bool   = i32;
type Dword  = u32;
type Long   = i32;
type Word   = u16;

const STD_INPUT_HANDLE:  Dword = 0xFFFFFFF6;
const STD_OUTPUT_HANDLE: Dword = 0xFFFFFFF5;

const ENABLE_VIRTUAL_TERMINAL_PROCESSING: Dword = 0x0004;
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: Dword = 0x00020016;
const EXTENDED_STARTUPINFO_PRESENT: Dword = 0x00080000;
const WAIT_OBJECT_0: Dword = 0;

#[repr(C)]
struct Coord { x: i16, y: i16 }

#[repr(C)]
struct SmallRect { left: i16, top: i16, right: i16, bottom: i16 }

#[repr(C)]
struct ScreenBufInfo {
    dw_size:                Coord,
    dw_cursor_position:     Coord,
    w_attributes:           Word,
    sr_window:              SmallRect,
    dw_maximum_window_size: Coord,
}

#[repr(C)]
#[allow(non_snake_case)]
struct ProcessInformation {
    h_process:    Handle,
    h_thread:     Handle,
    dw_process_id: Dword,
    dw_thread_id:  Dword,
}

/// Write a pointer value into a byte slice at the given offset (little-endian).
fn ptr_to_le(buf: &mut [u8], offset: usize, ptr: Handle) {
    let bytes = (ptr as usize).to_le_bytes();
    let end = (offset + bytes.len()).min(buf.len());
    let count = end - offset;
    buf[offset..end].copy_from_slice(&bytes[..count]);
}

unsafe extern "system" {
    fn GetStdHandle(n: Dword) -> Handle;
    fn GetConsoleMode(h: Handle, mode: *mut Dword) -> Bool;
    fn SetConsoleMode(h: Handle, mode: Dword) -> Bool;
    fn GetConsoleScreenBufferInfo(h: Handle, info: *mut ScreenBufInfo) -> Bool;
    fn CreatePseudoConsole(
        size: Coord, hIn: Handle, hOut: Handle, dwFlags: Dword, ppc: *mut Handle,
    ) -> Long;
    fn ResizePseudoConsole(pc: Handle, size: Coord) -> Long;
    fn ClosePseudoConsole(pc: Handle) -> Long;
    fn ReadFile(h: Handle, buf: *mut u8, n: Dword, read: *mut Dword, ov: *mut u8) -> Bool;
    fn WriteFile(h: Handle, buf: *const u8, n: Dword, written: *mut Dword, ov: *mut u8) -> Bool;
    fn WaitForSingleObject(h: Handle, ms: Dword) -> Dword;
    fn TerminateProcess(h: Handle, code: Dword) -> Bool;
    fn CloseHandle(h: Handle) -> Bool;
    fn CreateProcessW(
        app_name: *const u16, cmd_line: *mut u16,
        proc_attr: *mut u8, thread_attr: *mut u8,
        inherit_handles: Bool, creation_flags: Dword,
        env: *mut u8, cwd: *const u16,
        startup_info: *const u8, process_info: *mut u8,
    ) -> Bool;
    fn CreatePipe(
        read_handle: *mut Handle, write_handle: *mut Handle,
        attr: *mut u8, size: Dword,
    ) -> Bool;
    fn InitializeProcThreadAttributeList(
        attr: *mut u8, count: Dword, flags: Dword, size: *mut usize,
    ) -> Bool;
    fn UpdateProcThreadAttribute(
        attr: *mut u8, flags: Dword, attribute: Dword,
        value: *mut u8, size: usize, prev: *mut u8, ret_size: *mut u8,
    ) -> Bool;
    fn DeleteProcThreadAttributeList(attr: *mut u8);
}

// ── ConPTY backend ────────────────────────────────────────────────────────────

/// A persistent shell session hosted by a Windows Pseudo Console (ConPTY).
pub struct ConPtySession {
    pc:           Handle,
    in_read:      Handle,
    in_write:     Handle,
    out_read:     Handle,
    out_write:    Handle,
    process:      Handle,
    input_tx:     Sender<Vec<u8>>,
    input_thread: Option<thread::JoinHandle<()>>,
    closed:       bool,
}

impl ConPtySession {
    fn child_exited(&self) -> bool {
        unsafe { WaitForSingleObject(self.process, 0) == WAIT_OBJECT_0 }
    }

    fn try_wait(&mut self) -> Option<i32> {
        if self.child_exited() {
            self.closed = true;
            Some(0)
        } else {
            None
        }
    }

    fn kill(&mut self) -> bool {
        unsafe {
            let _ = TerminateProcess(self.process, 1);
        }
        true
    }

    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.input_tx
            .send(data.to_vec())
            .map_err(|_| "input channel disconnected".to_string())?;
        Ok(())
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), String> {
        unsafe {
            let size = Coord { x: cols as i16, y: rows as i16 };
            let hr = ResizePseudoConsole(self.pc, size);
            if hr != 0 {
                return Err(format!("ResizePseudoConsole failed: 0x{:X}", hr));
            }
        }
        Ok(())
    }
}

impl Drop for ConPtySession {
    fn drop(&mut self) {
        let _ = self.input_tx.send(Vec::new());
        if let Some(t) = self.input_thread.take() {
            let _ = t.join();
        }
        unsafe {
            if !self.pc.is_null() { let _ = ClosePseudoConsole(self.pc); }
            if !self.in_read.is_null() { let _ = CloseHandle(self.in_read); }
            if !self.in_write.is_null() { let _ = CloseHandle(self.in_write); }
            if !self.out_read.is_null() { let _ = CloseHandle(self.out_read); }
            if !self.out_write.is_null() { let _ = CloseHandle(self.out_write); }
            if !self.process.is_null() && !self.closed { let _ = CloseHandle(self.process); }
        }
    }
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn close_handle(h: Handle) {
    if !h.is_null() { let _ = CloseHandle(h); }
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn restore_modes(hin: Handle, hout: Handle, in_m: Dword, out_m: Dword) {
    let _ = SetConsoleMode(hin, in_m);
    let _ = SetConsoleMode(hout, out_m);
}

fn get_shell_path() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
}

/// Try to host a persistent shell via ConPTY. Returns None if creation fails.
fn try_spawn_conpty(
    command: &str,
    cwd: &str,
    tx: Sender<TerminalUpdate>,
) -> Option<ConPtySession> {
    unsafe {
        let hin = GetStdHandle(STD_INPUT_HANDLE);
        let hout = GetStdHandle(STD_OUTPUT_HANDLE);

        let mut in_mode: Dword = 0;
        let mut out_mode: Dword = 0;
        GetConsoleMode(hin, &mut in_mode);
        GetConsoleMode(hout, &mut out_mode);

        let new_in = in_mode & !(0x0002 | 0x0004 | 0x0001 | 0x0010 | 0x0008);
        SetConsoleMode(hin, new_in);
        let new_out = out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING | 0x0001;
        SetConsoleMode(hout, new_out);

        let mut info = std::mem::zeroed::<ScreenBufInfo>();
        GetConsoleScreenBufferInfo(hout, &mut info);
        let w = ((info.sr_window.right - info.sr_window.left + 1) as i16).max(80);
        let h = ((info.sr_window.bottom - info.sr_window.top + 1) as i16).max(24);
        let size = Coord { x: w, y: h };

        // ConPTY handles are passed with NULL attributes (non-inheritable),
        // and the virtual console is handed to the child via the attribute list.
        let mut in_read: Handle = std::ptr::null_mut();
        let mut in_write: Handle = std::ptr::null_mut();
        let mut out_read: Handle = std::ptr::null_mut();
        let mut out_write: Handle = std::ptr::null_mut();

        if CreatePipe(&mut in_read, &mut in_write, std::ptr::null_mut(), 0) == 0 {
            restore_modes(hin, hout, in_mode, out_mode);
            return None;
        }
        if CreatePipe(&mut out_read, &mut out_write, std::ptr::null_mut(), 0) == 0 {
            close_handle(in_read);
            close_handle(in_write);
            restore_modes(hin, hout, in_mode, out_mode);
            return None;
        }

        let mut pc_handle: Handle = std::ptr::null_mut();
        let hr = CreatePseudoConsole(size, in_read, out_write, 0, &mut pc_handle);
        if hr != 0 || pc_handle.is_null() {
            close_handle(in_read);
            close_handle(in_write);
            close_handle(out_read);
            close_handle(out_write);
            restore_modes(hin, hout, in_mode, out_mode);
            return None;
        }

        let shell = if command.trim().is_empty() {
            get_shell_path()
        } else {
            command.to_string()
        };
        let cmd_str = format!("\"{}\"", shell);
        let cmd_wide: Vec<u16> = cmd_str.encode_utf16().collect();
        let mut cmd_buf: Vec<u16> = cmd_wide.into_iter().chain(Some(0)).collect();

        let cwd_wide: Vec<u16> = cwd.encode_utf16().collect();
        let cwd_buf: Vec<u16> = cwd_wide.into_iter().chain(Some(0)).collect();

        // Attribute list carrying the pseudo console.
        let mut attr_size: usize = 0;
        let _ = InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attr_size);
        let mut attr_list = vec![0u8; attr_size];
        if InitializeProcThreadAttributeList(attr_list.as_mut_ptr(), 1, 0, &mut attr_size) == 0 {
            cleanup_conpty_failed(in_read, in_write, out_read, out_write);
            let _ = ClosePseudoConsole(pc_handle);
            restore_modes(hin, hout, in_mode, out_mode);
            return None;
        }
        if UpdateProcThreadAttribute(
            attr_list.as_mut_ptr(),
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            pc_handle as *mut u8,
            std::mem::size_of::<Handle>(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ) == 0 {
            DeleteProcThreadAttributeList(attr_list.as_mut_ptr());
            cleanup_conpty_failed(in_read, in_write, out_read, out_write);
            let _ = ClosePseudoConsole(pc_handle);
            restore_modes(hin, hout, in_mode, out_mode);
            return None;
        }

        // STARTUPINFOEXW (112 bytes on x64): STARTUPINFOW + lpAttributeList.
        let mut si = vec![0u8; 112];
        si[0..4].copy_from_slice(&112u32.to_le_bytes()); // cb = sizeof(STARTUPINFOEXW)
        ptr_to_le(&mut si, 104, attr_list.as_ptr() as *mut u8);

        let mut pi: ProcessInformation = std::mem::zeroed();
        let result = CreateProcessW(
            std::ptr::null(),
            cmd_buf.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0, // bInheritHandles: ConPTY provides std handles via the attribute
            EXTENDED_STARTUPINFO_PRESENT | 0x08000000, // + CREATE_NO_WINDOW
            std::ptr::null_mut(),
            cwd_buf.as_ptr(),
            si.as_ptr(),
            &mut pi as *mut _ as *mut u8,
        );

        DeleteProcThreadAttributeList(attr_list.as_mut_ptr());
        // The pseudo console holds a reference on its ends; close ours.
        close_handle(in_read);
        close_handle(out_write);

        if result == 0 {
            let _ = ClosePseudoConsole(pc_handle);
            close_handle(in_write);
            close_handle(out_read);
            restore_modes(hin, hout, in_mode, out_mode);
            return None;
        }

        // Input forwarding thread (writes queue via channel).
        let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let in_write_usize = in_write as usize;
        let input_thread = thread::spawn(move || {
            let in_write_h = in_write_usize as Handle;
            while let Ok(data) = input_rx.recv() {
                if data.is_empty() { break; }
                let mut written: Dword = 0;
                let _ = WriteFile(in_write_h, data.as_ptr(), data.len() as Dword, &mut written, std::ptr::null_mut());
            }
        });

        // Output reading thread with partial-line buffering.
        let out_read_usize = out_read as usize;
        let process_usize = pi.h_process as usize;
        thread::spawn(move || {
            let out_read_h = out_read_usize as Handle;
            let process_h = process_usize as Handle;
            let mut buf = [0u8; 4096];
            let mut residue = String::new();
            loop {
                let mut read: Dword = 0;
                let ret = ReadFile(out_read_h, buf.as_mut_ptr(), buf.len() as Dword, &mut read, std::ptr::null_mut());
                if ret == 0 || read == 0 {
                    if WaitForSingleObject(process_h, 0) == WAIT_OBJECT_0 { break; }
                    std::thread::sleep(std::time::Duration::from_micros(200));
                    continue;
                }
                residue.push_str(&String::from_utf8_lossy(&buf[..read as usize]));
                drain_lines(&mut residue, &tx);
            }
            drain_lines(&mut residue, &tx);
            let _ = tx.send(TerminalUpdate::Closed);
        });

        Some(ConPtySession {
            pc: pc_handle,
            in_read: std::ptr::null_mut(),  // closed above
            in_write,
            out_read,
            out_write: std::ptr::null_mut(), // closed above
            process: pi.h_process,
            input_tx,
            input_thread: Some(input_thread),
            closed: false,
        })
    }
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn cleanup_conpty_failed(
    in_read: Handle, in_write: Handle, out_read: Handle, out_write: Handle,
) {
    close_handle(in_read);
    close_handle(in_write);
    close_handle(out_read);
    close_handle(out_write);
}

/// Split complete lines out of `residue`, sending each as a TerminalUpdate::Line.
/// Retorna true se alguma linha terminada em newline foi emitida.
fn drain_lines(residue: &mut String, tx: &Sender<TerminalUpdate>) -> bool {
    let mut drained = false;
    while let Some(pos) = residue.find('\n') {
        let line: String = residue.drain(..=pos).collect();
        let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
        if !trimmed.is_empty() {
            let _ = tx.send(TerminalUpdate::Line(trimmed));
        }
        drained = true;
    }
    drained
}

/// Lê um stream de bytes de forma lossy (aceita codepages OEM não-UTF-8 sem
/// derrubar a thread) e emite `TerminalUpdate::Line` por linha quebrada.
///
/// Saída parcial sem newline (ex.: prompt `>>> ` de REPLs) é emitida após um
/// curto intervalo, para não ficar presa até o próximo newline.
fn read_lossy_stream<R: Read>(mut stream: R, tx: &Sender<TerminalUpdate>) {
    let mut buf = [0u8; 4096];
    let mut residue = String::new();
    let mut last_newline = std::time::Instant::now();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                residue.push_str(&String::from_utf8_lossy(&buf[..n]));
                if drain_lines(&mut residue, tx) {
                    last_newline = std::time::Instant::now();
                }
                // Saída parcial (prompt/progresso sem newline) — emite após
                // debounce para não fragmentar linhas normais.
                if !residue.is_empty()
                    && last_newline.elapsed() >= std::time::Duration::from_millis(50)
                {
                    let part = residue.trim_end().to_string();
                    if !part.is_empty() {
                        let _ = tx.send(TerminalUpdate::Line(part));
                    }
                    residue.clear();
                    last_newline = std::time::Instant::now();
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let tail = residue.trim().to_string();
    if !tail.is_empty() {
        let _ = tx.send(TerminalUpdate::Line(tail));
    }
}


// ── Pipe backend (fallback for hosts where ConPTY cannot attach) ──────────────

/// A persistent shell session driven through anonymous pipes.
///
/// Used when ConPTY hosting is unavailable (e.g. some automation hosts).
/// `powershell.exe -NoExit -Command -` keeps reading commands from stdin and
/// preserves shell state (cwd, environment) between writes.
pub struct PipeSession {
    child:          Option<Child>,
    input_tx:       Sender<Vec<u8>>,
    input_thread:   Option<thread::JoinHandle<()>>,
    closed:         bool,
}

impl PipeSession {
    fn try_wait(&mut self) -> Option<i32> {
        let code = self.child.as_mut()
            .and_then(|c| c.try_wait().ok().flatten())
            .map(|s| s.code().unwrap_or_default());
        if code.is_some() {
            self.closed = true;
        }
        code
    }

    fn kill(&mut self) -> bool {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
        true
    }

    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.input_tx
            .send(data.to_vec())
            .map_err(|_| "input channel disconnected".to_string())?;
        Ok(())
    }

    fn resize(&mut self, _cols: u16, _rows: u16) -> Result<(), String> {
        Ok(())
    }
}

impl Drop for PipeSession {
    fn drop(&mut self) {
        let _ = self.input_tx.send(Vec::new());
        if let Some(t) = self.input_thread.take() {
            let _ = t.join();
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_pipe(command: &str, cwd: &str, tx: Sender<TerminalUpdate>) -> Result<PipeSession, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // PowerShell with piped stdin is the most reliable persistent shell in a
    // non-console host. Fall back to the requested shell if PowerShell is
    // unavailable.
    let mut child = {
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", "-"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW);
        if let Ok(c) = cmd.spawn() {
            c
        } else {
            let shell = if command.trim().is_empty() {
                get_shell_path()
            } else {
                command.to_string()
            };
            let mut cmd = Command::new(&shell);
            cmd.arg("/Q")
                .current_dir(cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .creation_flags(CREATE_NO_WINDOW);
            cmd.spawn().map_err(|err| format!("failed to spawn {shell}: {err}"))?
        }
    };

    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    // Input forwarding thread.
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let input_thread = thread::spawn(move || {
        let mut stdin = stdin;
        while let Ok(data) = input_rx.recv() {
            if data.is_empty() { break; }
            let _ = stdin.write_all(&data);
            let _ = stdin.flush();
        }
    });

    // stdout reader (leitura lossy: saída pode vir em codepage OEM não-UTF-8).
    let tx_out = tx.clone();
    thread::spawn(move || {
        read_lossy_stream(stdout, &tx_out);
        let _ = tx_out.send(TerminalUpdate::Closed);
    });

    // stderr reader (leitura lossy).
    let tx_err = tx.clone();
    thread::spawn(move || {
        read_lossy_stream(stderr, &tx_err);
        let _ = tx_err.send(TerminalUpdate::Closed);
    });

    Ok(PipeSession {
        child: Some(child),
        input_tx,
        input_thread: Some(input_thread),
        closed: false,
    })
}

// ── Public platform surface ───────────────────────────────────────────────────

pub enum PlatformCommand {
    Conpty(ConPtySession),
    Pipe(PipeSession),
}

impl PlatformCommand {
    pub fn try_wait(&mut self) -> Option<i32> {
        match self {
            PlatformCommand::Conpty(s) => s.try_wait(),
            PlatformCommand::Pipe(s) => s.try_wait(),
        }
    }

    pub fn kill(&mut self) -> bool {
        match self {
            PlatformCommand::Conpty(s) => s.kill(),
            PlatformCommand::Pipe(s) => s.kill(),
        }
    }

    pub fn write(&mut self, data: &[u8]) -> Result<(), String> {
        match self {
            PlatformCommand::Conpty(s) => s.write(data),
            PlatformCommand::Pipe(s) => s.write(data),
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), String> {
        match self {
            PlatformCommand::Conpty(s) => s.resize(cols, rows),
            PlatformCommand::Pipe(s) => s.resize(cols, rows),
        }
    }
}

impl Drop for PlatformCommand {
    fn drop(&mut self) {
        // ConPtySession / PipeSession implement their own teardown.
    }
}

#[cfg(test)]
impl PlatformCommand {
    pub fn kind_label(&self) -> &'static str {
        match self {
            PlatformCommand::Conpty(_) => "conpty",
            PlatformCommand::Pipe(_) => "pipe",
        }
    }
}

/// Cache de viabilidade do ConPTY: em hosts onde um filho não consegue anexar
/// ao pseudo console (ex.: automação), o ConPTY fica quebrado permanentemente.
/// Detectar uma única vez evita o custo de reprobe a cada terminal aberto.
static CONPTY_VIABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Sonda se o ConPTY realmente anexa um filho neste host.
///
/// Cria um ConPTY descartável (com CREATE_NO_WINDOW, para não vazar o banner
/// do shell para o console real), envia `echo <marcador>` e verifica se o
/// marcador volta pela saída. Se não voltar, ConPTY está inoperante e devemos
/// usar o fallback por pipes.
fn conpty_is_viable(command: &str, cwd: &str) -> bool {
    const MARKER: &str = "manto_conpty_probe_4829";
    let (ptx, prx) = std::sync::mpsc::channel::<TerminalUpdate>();
    let mut probe = match try_spawn_conpty(command, cwd, ptx.clone()) {
        Some(p) => p,
        None => return false,
    };

    let line = format!("echo {MARKER}\r");
    if probe.write(line.as_bytes()).is_err() {
        return false;
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
    while std::time::Instant::now() < deadline {
        while let Ok(upd) = prx.try_recv() {
            if let TerminalUpdate::Line(l) = upd {
                if l.contains(MARKER) {
                    return true;
                }
            }
        }
        if probe.child_exited() {
            return false;
        }
        thread::sleep(std::time::Duration::from_millis(25));
    }
    false
}

/// Spawn a persistent shell session, preferring ConPTY and falling back to
/// anonymous pipes when the host cannot attach a child to a pseudo console.
pub fn spawn(command: &str, cwd: &str, tx: Sender<TerminalUpdate>) -> Result<PlatformCommand, String> {
    let viable = *CONPTY_VIABLE.get_or_init(|| conpty_is_viable(command, cwd));
    if viable {
        if let Some(conpty) = try_spawn_conpty(command, cwd, tx.clone()) {
            return Ok(PlatformCommand::Conpty(conpty));
        }
    }
    Ok(PlatformCommand::Pipe(spawn_pipe(command, cwd, tx)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conpty_probe_does_not_poison_following_pipe() {
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();

        fn probe_echo(idx: u32, cwd: &str) -> Result<(), String> {
            let (tx, rx) = std::sync::mpsc::channel::<TerminalUpdate>();
            let mut s = spawn("", cwd, tx).map_err(|e| format!("spawn[{idx}] err {e}"))?;
            let cmd = format!("echo POISONMARK{}\r", idx);
            s.write(cmd.as_bytes()).map_err(|e| e.to_string())?;
            let start = std::time::Instant::now();
            let mut got = Vec::new();
            while start.elapsed() < std::time::Duration::from_secs(5) {
                while let Ok(u) = rx.try_recv() {
                    if let TerminalUpdate::Line(l) = u { got.push(l.clone()); }
                }
                if got.iter().any(|l| l.contains(&format!("POISONMARK{idx}"))) {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            Err(format!("no marker[{idx}]; got={got:?}"))
        }

        // O primeiro spawn roda o probe ConPTY (cached). Verifica o segundo.
        let _ = probe_echo(1, &cwd);
        let r2 = probe_echo(2, &cwd);
        eprintln!("DEBUG second session result: {:?}", r2);
        assert!(r2.is_ok(), "second session after probe failed: {r2:?}");
    }

    #[test]
    fn pipe_fallback_persistent_session_works() {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use std::io::{BufRead, BufReader};
            let cwd = std::env::current_dir().unwrap();
            let cwd_s = cwd.to_string_lossy().to_string();

            let variants: [(&str, &[&str], u32); 3] = [
                ("cmd.exe", &["/Q"], 0x08000000),                       // CREATE_NO_WINDOW
                ("cmd.exe", &["/Q"], 0x08000000 | 0x00000100),          // + CREATE_NEW_CONSOLE
                ("powershell.exe", &["-NoLogo", "-NoExit", "-Command", "-"], 0x08000000),
            ];

            for (i, (prog, args, flags)) in variants.iter().enumerate() {
                let (tx2, rx2) = std::sync::mpsc::channel::<Vec<u8>>();
                let mut cmd = std::process::Command::new(prog);
                cmd.args(*args)
                    .current_dir(&cwd_s)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .creation_flags(*flags);
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => { eprintln!("variant {i} spawn err: {e}"); continue; }
                };
                let mut stdin = child.stdin.take().unwrap();
                let stdout = child.stdout.take().unwrap();
                let mut reader = BufReader::new(stdout);
                let th = thread::spawn(move || {
                    while let Ok(data) = rx2.recv() {
                        if data.is_empty() { break; }
                        let _ = stdin.write_all(&data);
                        let _ = stdin.flush();
                    }
                });
                std::thread::sleep(std::time::Duration::from_millis(300));
                let _ = tx2.send(b"echo PITEST5511\r\n".to_vec());
                let start = std::time::Instant::now();
                let mut all = String::new();
                let mut line = String::new();
                while start.elapsed() < std::time::Duration::from_secs(3) {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => all.push_str(&line),
                        Err(_) => break,
                    }
                    if all.contains("PITEST5511") { break; }
                }
                let _ = tx2.send(Vec::new());
                let _ = th.join();
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("variant {i} ({prog} flags={flags}): output={:?}", all);
                if all.contains("PITEST5511") {
                    return;
                }
            }
            panic!("no launch variant worked");
        }
        #[cfg(not(windows))]
        {
            let _ = (super::spawn, "noop");
        }
    }
}
