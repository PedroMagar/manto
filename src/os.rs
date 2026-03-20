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

pub struct Writer(io::BufWriter<io::Stdout>);

impl Writer {
    pub fn new() -> Self {
        Self(io::BufWriter::new(io::stdout()))
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
    Backspace,
    Escape,
    End,
    Home,
    PageUp,
    PageDown,
    CtrlC,
    CtrlEnter,
    CtrlT,
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
        loop {
            let mut buf = [0u8; 1];
            std::io::stdin().read_exact(&mut buf).unwrap();
            match buf[0] {
                3        => return Key::CtrlC,
                20       => return Key::CtrlT,
                8 | 127  => return Key::Backspace,
                13       => return Key::Enter,
                27 => {
                    if poll(10) {
                        let mut seq = [0u8; 2];
                        std::io::stdin().read_exact(&mut seq).unwrap();
                        if seq[0] == b'[' {
                            match seq[1] {
                                b'A' => return Key::Up,
                                b'B' => return Key::Down,
                                b'C' => return Key::Right,
                                b'D' => return Key::Left,
                                b'F' => return Key::End,
                                b'H' => return Key::Home,
                                b'5' | b'6' => {
                                    let mut term = [0u8; 1];
                                    std::io::stdin().read_exact(&mut term).unwrap();
                                    if term[0] == b'~' {
                                        if seq[1] == b'5' { return Key::PageUp; }
                                        else { return Key::PageDown; }
                                    }
                                    continue;
                                }
                                _    => continue,
                            }
                        }
                    } else {
                        return Key::Escape;
                    }
                }
                b if b.is_ascii_graphic() || b == b' ' => return Key::Char(buf[0] as char),
                _ => continue,
            }
        }
    }
}

// ─── Windows ─────────────────────────────────────────────────────────────────
// Usa ReadConsoleInputW em vez de ReadFile+VT para evitar bloqueio:
// WaitForSingleObject sinaliza para qualquer evento de console (foco, etc.),
// enquanto ReadFile fica bloqueado aguardando bytes VT que nunca chegam.
// Com ReadConsoleInputW lemos registros diretamente e descartamos não-teclado.
#[cfg(windows)]
mod platform {
    use super::Key;
    use std::time::{Duration, Instant};

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
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: Dword = 0x0004;
    const ENABLE_PROCESSED_OUTPUT:            Dword = 0x0001;

    const WAIT_OBJECT_0:   Dword = 0;
    const KEY_EVENT_TYPE:  Word  = 0x0001;
    const LEFT_CTRL:       Dword = 0x0008;
    const RIGHT_CTRL:      Dword = 0x0004;

    // INPUT_RECORD: WORD EventType (2) + WORD pad (2) + union Event (16 bytes)
    #[repr(C)]
    struct InputRecord { event_type: Word, _pad: Word, event: [u8; 16] }

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
        fn ReadConsoleInputW(h: Handle, buf: *mut InputRecord, len: Dword, read: *mut Dword) -> Bool;
        fn PeekConsoleInputW(h: Handle, buf: *mut InputRecord, len: Dword, read: *mut Dword) -> Bool;
        fn GetNumberOfConsoleInputEvents(h: Handle, count: *mut Dword) -> Bool;
        fn GetKeyState(n_virt_key: i32) -> i16;
    }

    static mut ORIG_IN_MODE:  Dword = 0;
    static mut ORIG_OUT_MODE: Dword = 0;

    // Helpers para ler campos do KEY_EVENT_RECORD dentro de event: [u8; 16]
    // KEY_EVENT_RECORD layout: bKeyDown(i32@0) wRepeat(u16@4) wVK(u16@6)
    //   wScan(u16@8) uChar/WCHAR(u16@10) dwCtrl(u32@12)
    fn ke_key_down(e: &[u8; 16]) -> bool { i32::from_ne_bytes([e[0],e[1],e[2],e[3]]) != 0 }
    fn ke_vk(e: &[u8; 16])       -> u16  { u16::from_ne_bytes([e[6], e[7]]) }
    fn ke_char(e: &[u8; 16])     -> u16  { u16::from_ne_bytes([e[10],e[11]]) }
    fn ke_ctrl(e: &[u8; 16])     -> u32  { u32::from_ne_bytes([e[12],e[13],e[14],e[15]]) }

    fn is_key_down(rec: &InputRecord) -> bool {
        rec.event_type == KEY_EVENT_TYPE && ke_key_down(&rec.event)
    }

    pub fn enable_raw_mode() {
        unsafe {
            let hin  = GetStdHandle(STD_INPUT_HANDLE);
            let hout = GetStdHandle(STD_OUTPUT_HANDLE);
            GetConsoleMode(hin,  &raw mut ORIG_IN_MODE);
            GetConsoleMode(hout, &raw mut ORIG_OUT_MODE);

            // Sem VT input: usamos ReadConsoleInputW e lemos registros diretamente
            let new_in = ORIG_IN_MODE
                & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT
                    | ENABLE_MOUSE_INPUT | ENABLE_WINDOW_INPUT);
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

    /// Drena eventos não-KEY_DOWN da fila. Retorna true se sobrou um KEY_DOWN.
    fn drain_non_key(hin: Handle) -> bool {
        unsafe {
            loop {
                let mut count = 0u32;
                GetNumberOfConsoleInputEvents(hin, &mut count);
                if count == 0 { return false; }

                let mut rec = std::mem::zeroed::<InputRecord>();
                let mut peeked = 0u32;
                PeekConsoleInputW(hin, &mut rec, 1, &mut peeked);
                if peeked == 0 { return false; }

                if is_key_down(&rec) { return true; }

                // Descarta evento inútil (key up, mouse, foco, etc.)
                let mut read = 0u32;
                ReadConsoleInputW(hin, &mut rec, 1, &mut read);
            }
        }
    }

    /// Retorna true se houver KEY_DOWN disponível dentro do timeout.
    pub fn poll(timeout_ms: u64) -> bool {
        unsafe {
            let hin = GetStdHandle(STD_INPUT_HANDLE);
            if drain_non_key(hin) { return true; }

            let deadline = Instant::now() + Duration::from_millis(timeout_ms);
            loop {
                let now = Instant::now();
                if now >= deadline { return false; }
                let rem = (deadline - now).as_millis().min(50) as Dword;

                if WaitForSingleObject(hin, rem) == WAIT_OBJECT_0 {
                    if drain_non_key(hin) { return true; }
                } else {
                    return false;
                }
            }
        }
    }

    pub fn read_key() -> Key {
        unsafe {
            let hin = GetStdHandle(STD_INPUT_HANDLE);
            loop {
                let mut rec = std::mem::zeroed::<InputRecord>();
                let mut read = 0u32;
                ReadConsoleInputW(hin, &mut rec, 1, &mut read);
                if read == 0 || !is_key_down(&rec) { continue; }

                let vk   = ke_vk(&rec.event);
                let ch   = ke_char(&rec.event);
                let ctrl = ke_ctrl(&rec.event) & (LEFT_CTRL | RIGHT_CTRL) != 0;

                if ch == 0x03 || (ctrl && vk == 0x43) { return Key::CtrlC; }
                if ctrl && vk == 0x54 { return Key::CtrlT; }

                // Para Ctrl+Enter usamos GetKeyState (estado real-time) porque
                // dwControlKeyState pode não reportar Ctrl corretamente neste contexto.
                if vk == 0x0D {
                    let ctrl_held = ctrl || (GetKeyState(0x11) as u16 & 0x8000 != 0);
                    if ctrl_held { return Key::CtrlEnter; }
                    return Key::Enter;
                }

                match vk {
                    0x08 => return Key::Backspace,
                    0x1B => return Key::Escape,
                    0x21 => return Key::PageUp,
                    0x22 => return Key::PageDown,
                    0x23 => return Key::End,
                    0x24 => return Key::Home,
                    0x26 => return Key::Up,
                    0x28 => return Key::Down,
                    0x25 => return Key::Left,
                    0x27 => return Key::Right,
                    _ => {}
                }

                if let Some(c) = char::from_u32(ch as u32) {
                    if c.is_ascii_graphic() || c == ' ' { return Key::Char(c); }
                }
            }
        }
    }
}
