// os.rs — Camada de sistema operacional.
//
// Contém tudo que depende do OS concreto:
//   - Writer  : saída (stdout agora; framebuffer/serial no seu OS)
//   - Clock   : tempo (Instant agora; registrador de hardware no seu OS)
//   - Key     : eventos de teclado
//   - platform: raw mode, tamanho do terminal, polling e leitura de input
//
// Para portar para um novo OS, substitua este arquivo mantendo as mesmas
// interfaces públicas. Os demais arquivos (terminal.rs, gui.rs, pointer.rs)
// não precisam ser alterados.

use std::io::{self, Write};
use std::time::{Duration, Instant};

// ── Writer ────────────────────────────────────────────────────────────────────
// No seu OS: escreva para um framebuffer de texto, porta serial, etc.

pub struct Writer(io::Stdout);

impl Writer {
    pub fn new() -> Self {
        Self(io::stdout())
    }
}

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

// ── Clock ─────────────────────────────────────────────────────────────────────
// No seu OS: leia um registrador de hardware (TSC, RTC, timer MMIO, etc.)

pub struct Clock(Instant);

impl Clock {
    pub fn now() -> Self {
        Self(Instant::now())
    }

    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

// ── Key ───────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Enter,
    CtrlC,
    Unknown,
}

// ── Plataforma ────────────────────────────────────────────────────────────────
//
// Interface que cada plataforma deve exportar:
//   enable_raw_mode()      — coloca terminal em raw mode
//   disable_raw_mode()     — restaura modo original
//   size() -> (u16, u16)   — (largura, altura) em células
//   poll(ms: u64) -> bool  — true se há input disponível dentro do timeout
//   read_key() -> Key      — lê e decodifica a próxima tecla

pub use platform::{enable_raw_mode, disable_raw_mode, size, poll, read_key};

// ─── Unix (Linux, macOS, Redox) ──────────────────────────────────────────────
#[cfg(unix)]
mod platform {
    use super::Key;
    use std::io::Read;
    use std::mem::MaybeUninit;

    static mut ORIG_TERMIOS: Option<libc::termios> = None;

    pub fn enable_raw_mode() {
        unsafe {
            let mut t = MaybeUninit::<libc::termios>::uninit();
            libc::tcgetattr(libc::STDIN_FILENO, t.as_mut_ptr());
            let t = t.assume_init();
            ORIG_TERMIOS = Some(t);

            let mut raw = t;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
            raw.c_iflag &= !(libc::IXON);
            raw.c_oflag &= !(libc::OPOST);
            raw.c_cc[libc::VMIN as usize]  = 1;
            raw.c_cc[libc::VTIME as usize] = 0;
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
        }
    }

    pub fn disable_raw_mode() {
        unsafe {
            if let Some(orig) = ORIG_TERMIOS {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &orig);
            }
        }
    }

    pub fn size() -> (u16, u16) {
        unsafe {
            let mut ws = MaybeUninit::<libc::winsize>::uninit();
            libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, ws.as_mut_ptr());
            let ws = ws.assume_init();
            (ws.ws_col, ws.ws_row)
        }
    }

    pub fn poll(timeout_ms: u64) -> bool {
        unsafe {
            let mut fds = [libc::pollfd {
                fd:      libc::STDIN_FILENO,
                events:  libc::POLLIN,
                revents: 0,
            }];
            libc::poll(fds.as_mut_ptr(), 1, timeout_ms as libc::c_int) > 0
        }
    }

    pub fn read_key() -> Key {
        let mut buf = [0u8; 1];
        std::io::stdin().read_exact(&mut buf).unwrap();
        match buf[0] {
            3  => Key::CtrlC,
            13 => Key::Enter,
            27 => {
                if poll(10) {
                    let mut seq = [0u8; 2];
                    std::io::stdin().read_exact(&mut seq).unwrap();
                    if seq[0] == b'[' {
                        match seq[1] {
                            b'A' => Key::Up,
                            b'B' => Key::Down,
                            b'C' => Key::Right,
                            b'D' => Key::Left,
                            _    => Key::Unknown,
                        }
                    } else {
                        Key::Unknown
                    }
                } else {
                    Key::Unknown
                }
            }
            b if b.is_ascii() => Key::Char(b as char),
            _ => Key::Unknown,
        }
    }
}

// ─── Windows ─────────────────────────────────────────────────────────────────
#[cfg(windows)]
mod platform {
    use super::Key;

    type Handle = *mut u8;
    type Bool   = i32;
    type Dword  = u32;
    type Short  = i16;
    type Word   = u16;

    const STD_INPUT_HANDLE:  Dword = 0xFFFFFFF6;
    const STD_OUTPUT_HANDLE: Dword = 0xFFFFFFF5;

    const ENABLE_LINE_INPUT:                  Dword = 0x0002;
    const ENABLE_ECHO_INPUT:                  Dword = 0x0004;
    const ENABLE_PROCESSED_INPUT:             Dword = 0x0001;
    const ENABLE_MOUSE_INPUT:                 Dword = 0x0010;
    const ENABLE_WINDOW_INPUT:                Dword = 0x0008;
    const ENABLE_VIRTUAL_TERMINAL_INPUT:      Dword = 0x0200;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: Dword = 0x0004;
    const ENABLE_PROCESSED_OUTPUT:            Dword = 0x0001;

    const WAIT_OBJECT_0: Dword = 0;

    #[repr(C)] struct Coord        { x: Short, y: Short }
    #[repr(C)] struct SmallRect    { left: Short, top: Short, right: Short, bottom: Short }
    #[repr(C)] struct ScreenBufInfo {
        dw_size:                Coord,
        dw_cursor_position:     Coord,
        w_attributes:           Word,
        sr_window:              SmallRect,
        dw_maximum_window_size: Coord,
    }

    unsafe extern "system" {
        fn GetStdHandle(n: Dword) -> Handle;
        fn GetConsoleMode(h: Handle, mode: *mut Dword) -> Bool;
        fn SetConsoleMode(h: Handle, mode: Dword)      -> Bool;
        fn GetConsoleScreenBufferInfo(h: Handle, info: *mut ScreenBufInfo) -> Bool;
        fn WaitForSingleObject(h: Handle, ms: Dword)   -> Dword;
        fn ReadFile(h: Handle, buf: *mut u8, to_read: Dword, read: *mut Dword, overlapped: *mut u8) -> Bool;
    }

    static mut ORIG_IN_MODE:  Dword = 0;
    static mut ORIG_OUT_MODE: Dword = 0;

    pub fn enable_raw_mode() {
        unsafe {
            let hin  = GetStdHandle(STD_INPUT_HANDLE);
            let hout = GetStdHandle(STD_OUTPUT_HANDLE);
            GetConsoleMode(hin,  &raw mut ORIG_IN_MODE);
            GetConsoleMode(hout, &raw mut ORIG_OUT_MODE);

            // Desativa mouse e window events para evitar que o handle sinalize
            // sem ter dados de teclado, o que causaria bloqueio no ReadFile.
            let new_in = (ORIG_IN_MODE
                & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT
                    | ENABLE_MOUSE_INPUT | ENABLE_WINDOW_INPUT))
                | ENABLE_VIRTUAL_TERMINAL_INPUT;
            SetConsoleMode(hin, new_in);

            let new_out = ORIG_OUT_MODE
                | ENABLE_VIRTUAL_TERMINAL_PROCESSING
                | ENABLE_PROCESSED_OUTPUT;
            SetConsoleMode(hout, new_out);
        }
    }

    pub fn disable_raw_mode() {
        unsafe {
            let hin  = GetStdHandle(STD_INPUT_HANDLE);
            let hout = GetStdHandle(STD_OUTPUT_HANDLE);
            SetConsoleMode(hin,  ORIG_IN_MODE);
            SetConsoleMode(hout, ORIG_OUT_MODE);
        }
    }

    pub fn size() -> (u16, u16) {
        unsafe {
            let hout = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut info = std::mem::zeroed::<ScreenBufInfo>();
            GetConsoleScreenBufferInfo(hout, &mut info);
            let w = (info.sr_window.right  - info.sr_window.left + 1) as u16;
            let h = (info.sr_window.bottom - info.sr_window.top  + 1) as u16;
            (w, h)
        }
    }

    pub fn poll(timeout_ms: u64) -> bool {
        unsafe {
            let hin = GetStdHandle(STD_INPUT_HANDLE);
            WaitForSingleObject(hin, timeout_ms as Dword) == WAIT_OBJECT_0
        }
    }

    fn read_byte() -> u8 {
        unsafe {
            let hin = GetStdHandle(STD_INPUT_HANDLE);
            let mut byte = 0u8;
            let mut read = 0u32;
            ReadFile(hin, &mut byte, 1, &mut read, std::ptr::null_mut());
            byte
        }
    }

    pub fn read_key() -> Key {
        match read_byte() {
            3  => Key::CtrlC,
            13 => Key::Enter,
            27 => {
                if poll(10) {
                    let b1 = read_byte();
                    if b1 == b'[' && poll(10) {
                        match read_byte() {
                            b'A' => Key::Up,
                            b'B' => Key::Down,
                            b'C' => Key::Right,
                            b'D' => Key::Left,
                            _    => Key::Unknown,
                        }
                    } else {
                        Key::Unknown
                    }
                } else {
                    Key::Unknown
                }
            }
            b if b.is_ascii() => Key::Char(b as char),
            _ => Key::Unknown,
        }
    }
}
