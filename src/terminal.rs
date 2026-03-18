// terminal.rs — Camada de abstração de terminal usando ANSI/VT100 puro.
//
// Para portar para um novo OS, implemente o módulo `platform` abaixo
// com as mesmas funções públicas: enable_raw_mode, disable_raw_mode,
// size, poll, read_key.

use std::io::Write;

// ── Sequências ANSI ──────────────────────────────────────────────────────────

pub fn clear(out: &mut impl Write) {
    write!(out, "\x1b[2J\x1b[H").unwrap();
}

pub fn move_to(out: &mut impl Write, x: u16, y: u16) {
    // ANSI usa 1-based (linha;coluna)
    write!(out, "\x1b[{};{}H", y + 1, x + 1).unwrap();
}

pub fn hide_cursor(out: &mut impl Write) {
    write!(out, "\x1b[?25l").unwrap();
}

pub fn show_cursor(out: &mut impl Write) {
    write!(out, "\x1b[?25h").unwrap();
}

pub fn enter_alt_screen(out: &mut impl Write) {
    write!(out, "\x1b[?1049h").unwrap();
}

pub fn leave_alt_screen(out: &mut impl Write) {
    write!(out, "\x1b[?1049l").unwrap();
}

// ── Cores (ANSI SGR) ─────────────────────────────────────────────────────────

pub const RESET:        &str = "\x1b[0m";
pub const FG_BLACK:     &str = "\x1b[30m";
pub const FG_WHITE:     &str = "\x1b[37m";
pub const FG_GREEN:     &str = "\x1b[32m";
pub const BG_WHITE:     &str = "\x1b[47m";
pub const BG_DARK_GREY: &str = "\x1b[100m";

/// Imprime texto com cor de frente e fundo, depois reseta.
pub fn print_styled(out: &mut impl Write, text: &str, fg: &str, bg: &str) {
    write!(out, "{}{}{}{}", fg, bg, text, RESET).unwrap();
}

// ── Eventos de teclado ───────────────────────────────────────────────────────

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

// ── Camada de plataforma ─────────────────────────────────────────────────────
//
// Cada plataforma exporta:
//   enable_raw_mode()        — coloca terminal em raw mode
//   disable_raw_mode()       — restaura modo original
//   size() -> (u16, u16)     — (largura, altura) em células
//   poll(timeout_ms: u64)    — true se há input disponível
//   read_key() -> Key        — lê e decodifica próxima tecla

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
                // Sequência de escape — tenta ler mais bytes
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

    // Flags de modo do console
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

            // Desativa line input, echo, processed input, mouse e window events.
            // Mouse e window events também sinalizam o handle e causariam bloqueio
            // no ReadFile esperando um byte de teclado que nunca chega.
            let new_in = (ORIG_IN_MODE
                & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT
                    | ENABLE_MOUSE_INPUT | ENABLE_WINDOW_INPUT))
                | ENABLE_VIRTUAL_TERMINAL_INPUT;
            SetConsoleMode(hin, new_in);

            // Ativa processamento ANSI na saída
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

    /// Lê um byte direto do handle do console, sem passar pelo buffer da CRT.
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
                // Sequência de escape — tenta ler mais bytes
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
