// Input uses ReadConsoleInputW instead of ReadFile+VT to avoid blocking:
// WaitForSingleObject signals on any console event (focus, etc.), while
// ReadFile would block waiting for VT bytes that never arrive. With
// ReadConsoleInputW records are read directly and non-keyboard events are
// discarded.

use super::Key;
use std::time::{Duration, Instant};

type Handle = *mut u8;
type Bool = i32;
type Dword = u32;
type Short = i16;
type Word = u16;

const STD_INPUT_HANDLE: Dword = 0xFFFFFFF6;
const STD_OUTPUT_HANDLE: Dword = 0xFFFFFFF5;

const ENABLE_LINE_INPUT: Dword = 0x0002;
const ENABLE_ECHO_INPUT: Dword = 0x0004;
const ENABLE_PROCESSED_INPUT: Dword = 0x0001;
const ENABLE_MOUSE_INPUT: Dword = 0x0010;
const ENABLE_WINDOW_INPUT: Dword = 0x0008;
const ENABLE_VIRTUAL_TERMINAL_PROCESSING: Dword = 0x0004;
const ENABLE_PROCESSED_OUTPUT: Dword = 0x0001;

const WAIT_OBJECT_0: Dword = 0;
const KEY_EVENT_TYPE: Word = 0x0001;
const MOUSE_EVENT_TYPE: Word = 0x0002;
const LEFT_CTRL: Dword = 0x0008;
const RIGHT_CTRL: Dword = 0x0004;
const LEFT_ALT: Dword = 0x0002;
const RIGHT_ALT: Dword = 0x0001;
const VK_SHIFT: i32 = 0x10;
const GMEM_MOVEABLE: Dword = 0x0002;
const CF_UNICODETEXT: Dword = 13;

// MOUSE_EVENT_RECORD event flags.
const MOUSE_MOVED: Dword = 0x0001;
const MOUSE_WHEELED: Dword = 0x0004;

// MOUSE_EVENT_RECORD dwButtonState bits.
const FROM_LEFT_1ST_BUTTON_PRESSED: Dword = 0x0001;
const RIGHTMOST_BUTTON_PRESSED: Dword = 0x0002;
const FROM_LEFT_2ND_BUTTON_PRESSED: Dword = 0x0004;

// MOUSE_EVENT_RECORD dwControlKeyState bits.
const SHIFT_PRESSED: Dword = 0x0010;

// GetKeyState/GetAsyncKeyState-style: ctrl/alt are read from key state like
// the keyboard path, so we only need SHIFT here; ctrl/alt use VK values.
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;

// INPUT_RECORD: WORD EventType (2) + WORD pad (2) + union Event (16 bytes)
#[repr(C)]
struct InputRecord {
    event_type: Word,
    _pad: Word,
    event: [u8; 16],
}

#[repr(C)]
struct Coord {
    x: Short,
    y: Short,
}
#[repr(C)]
struct SmallRect {
    left: Short,
    top: Short,
    right: Short,
    bottom: Short,
}
#[repr(C)]
struct ScreenBufInfo {
    dw_size: Coord,
    dw_cursor_position: Coord,
    w_attributes: Word,
    sr_window: SmallRect,
    dw_maximum_window_size: Coord,
}

unsafe extern "system" {
    fn GetStdHandle(n: Dword) -> Handle;
    fn GetConsoleMode(h: Handle, mode: *mut Dword) -> Bool;
    fn SetConsoleMode(h: Handle, mode: Dword) -> Bool;
    fn GetConsoleScreenBufferInfo(h: Handle, info: *mut ScreenBufInfo) -> Bool;
    fn WaitForSingleObject(h: Handle, ms: Dword) -> Dword;
    fn ReadConsoleInputW(h: Handle, buf: *mut InputRecord, len: Dword, read: *mut Dword) -> Bool;
    #[cfg(test)]
    fn WriteConsoleInputW(h: Handle, buf: *const InputRecord, len: Dword, read: *mut Dword)
    -> Bool;
    fn PeekConsoleInputW(h: Handle, buf: *mut InputRecord, len: Dword, read: *mut Dword) -> Bool;
    fn GetNumberOfConsoleInputEvents(h: Handle, count: *mut Dword) -> Bool;
    fn GetKeyState(n_virt_key: i32) -> i16;
}

unsafe extern "system" {
    fn OpenClipboard(h: Handle) -> Bool;
    fn EmptyClipboard() -> Bool;
    fn SetClipboardData(u_format: Dword, h_mem: Handle) -> Handle;
    fn GetClipboardData(u_format: Dword) -> Handle;
    fn CloseClipboard() -> Bool;
    fn GlobalAlloc(u_flags: Dword, dw_bytes: usize) -> Handle;
    fn GlobalLock(h_mem: Handle) -> *mut u8;
    fn GlobalUnlock(h_mem: Handle) -> Bool;
    fn GlobalFree(h_mem: Handle) -> Handle;
    fn GlobalSize(h_mem: Handle) -> usize;
}

static mut ORIG_IN_MODE: Dword = 0;
static mut ORIG_OUT_MODE: Dword = 0;

// Helpers to read KEY_EVENT_RECORD fields out of event: [u8; 16].
// KEY_EVENT_RECORD layout: bKeyDown(i32@0) wRepeat(u16@4) wVK(u16@6)
//   wScan(u16@8) uChar/WCHAR(u16@10) dwCtrl(u32@12)
fn ke_key_down(e: &[u8; 16]) -> bool {
    i32::from_ne_bytes([e[0], e[1], e[2], e[3]]) != 0
}
fn ke_vk(e: &[u8; 16]) -> u16 {
    u16::from_ne_bytes([e[6], e[7]])
}
fn ke_char(e: &[u8; 16]) -> u16 {
    u16::from_ne_bytes([e[10], e[11]])
}
fn ke_ctrl(e: &[u8; 16]) -> u32 {
    u32::from_ne_bytes([e[12], e[13], e[14], e[15]])
}

// MOUSE_EVENT_RECORD layout (inside the 16-byte event union):
//   dwMousePosition   COORD (Short x@0, Short y@2)
//   dwButtonState     DWORD @4
//   dwControlKeyState DWORD @8
//   dwEventFlags      DWORD @12
fn me_x(e: &[u8; 16]) -> u16 {
    i16::from_ne_bytes([e[0], e[1]]) as u16
}
fn me_y(e: &[u8; 16]) -> u16 {
    i16::from_ne_bytes([e[2], e[3]]) as u16
}
fn me_button(e: &[u8; 16]) -> u32 {
    u32::from_ne_bytes([e[4], e[5], e[6], e[7]])
}
fn me_ctrl(e: &[u8; 16]) -> u32 {
    u32::from_ne_bytes([e[8], e[9], e[10], e[11]])
}
fn me_flags(e: &[u8; 16]) -> u32 {
    u32::from_ne_bytes([e[12], e[13], e[14], e[15]])
}

fn is_key_down(rec: &InputRecord) -> bool {
    rec.event_type == KEY_EVENT_TYPE && ke_key_down(&rec.event)
}

fn is_mouse(rec: &InputRecord) -> bool {
    rec.event_type == MOUSE_EVENT_TYPE
}

/// Decode a MOUSE_EVENT_RECORD into an `os::Key::Mouse`. Coordinates are
/// 0-based from the console; Manto treats the terminal as the whole screen.
fn decode_mouse(rec: &InputRecord) -> Key {
    use super::{MouseAction, MouseButton, MouseEvent};

    let e = &rec.event;
    let x = me_x(e).saturating_add(1);
    let y = me_y(e).saturating_add(1);
    let button_state = me_button(e);
    let ctrl_state = me_ctrl(e);
    let flags = me_flags(e);

    // Ctrl/Alt are read live like the keyboard path (this context can lie).
    let ctrl = unsafe { GetKeyState(VK_CONTROL) as u16 & 0x8000 != 0 };
    let alt = unsafe { GetKeyState(VK_MENU) as u16 & 0x8000 != 0 };
    let shift = ctrl_state & SHIFT_PRESSED != 0;

    let pressed = |btn| MouseEvent {
        x,
        y,
        kind: MouseAction::Press,
        button: btn,
        shift,
        ctrl,
        alt,
    };

    if flags & MOUSE_WHEELED != 0 {
        let delta = ((button_state >> 16) as u16) as i16;
        let button = if delta > 0 {
            MouseButton::WheelUp
        } else {
            MouseButton::WheelDown
        };
        return Key::Mouse(pressed(button));
    }

    if flags & MOUSE_MOVED != 0 {
        let held = button_state
            & (FROM_LEFT_1ST_BUTTON_PRESSED
                | RIGHTMOST_BUTTON_PRESSED
                | FROM_LEFT_2ND_BUTTON_PRESSED);
        if held != 0 {
            let button = if held & FROM_LEFT_1ST_BUTTON_PRESSED != 0 {
                MouseButton::Left
            } else if held & RIGHTMOST_BUTTON_PRESSED != 0 {
                MouseButton::Right
            } else {
                MouseButton::Middle
            };
            return Key::Mouse(MouseEvent {
                x,
                y,
                kind: MouseAction::Drag,
                button,
                shift,
                ctrl,
                alt,
            });
        }
        return Key::Mouse(MouseEvent {
            x,
            y,
            kind: MouseAction::Move,
            button: MouseButton::Left,
            shift,
            ctrl,
            alt,
        });
    }

    if button_state & FROM_LEFT_1ST_BUTTON_PRESSED != 0 {
        return Key::Mouse(pressed(MouseButton::Left));
    }
    if button_state & RIGHTMOST_BUTTON_PRESSED != 0 {
        return Key::Mouse(pressed(MouseButton::Right));
    }
    if button_state & FROM_LEFT_2ND_BUTTON_PRESSED != 0 {
        return Key::Mouse(pressed(MouseButton::Middle));
    }

    // No button and no flags: a button just went up.
    Key::Mouse(MouseEvent {
        x,
        y,
        kind: MouseAction::Release,
        button: MouseButton::Left,
        shift,
        ctrl,
        alt,
    })
}

pub fn enable_raw_mode() {
    unsafe {
        let hin = GetStdHandle(STD_INPUT_HANDLE);
        let hout = GetStdHandle(STD_OUTPUT_HANDLE);
        GetConsoleMode(hin, &raw mut ORIG_IN_MODE);
        GetConsoleMode(hout, &raw mut ORIG_OUT_MODE);

        // No VT input: records are read directly via ReadConsoleInputW.
        // Mouse stays enabled so MOUSE_EVENT_RECORDs reach read_key.
        let new_in = ORIG_IN_MODE
            & !(ENABLE_LINE_INPUT
                | ENABLE_ECHO_INPUT
                | ENABLE_PROCESSED_INPUT
                | ENABLE_WINDOW_INPUT)
            | ENABLE_MOUSE_INPUT;
        SetConsoleMode(hin, new_in);

        let new_out = ORIG_OUT_MODE | ENABLE_VIRTUAL_TERMINAL_PROCESSING | ENABLE_PROCESSED_OUTPUT;
        SetConsoleMode(hout, new_out);
    }

    // Ask the host terminal to report pointer events: without these DEC modes
    // a VT host (e.g. Windows Terminal) never sends mouse, so only the
    // physical-console path (real conhost window clicks) would generate
    // MOUSE_EVENT_RECORDs. With VT processing on, these are interpreted by
    // the terminal and ConPTY relays the reports back as INPUT_RECORDs.
    write_mouse_mode(true);
}

pub fn disable_raw_mode() {
    // Tell the host to stop reporting pointers before restoring the console
    // (after the restore the VT escapes would be printed literally).
    write_mouse_mode(false);
    unsafe {
        let hin = GetStdHandle(STD_INPUT_HANDLE);
        let hout = GetStdHandle(STD_OUTPUT_HANDLE);
        SetConsoleMode(hin, ORIG_IN_MODE);
        SetConsoleMode(hout, ORIG_OUT_MODE);
    }
}

/// Enable (`true`) / disable (`false`) DEC mouse tracking on stdout: X10
/// clicks (1000), press+drag (1002), any-motion hover (1003) and SGR (1006) so
/// coordinates stay exact. Ignored by hosts without mouse-reporting support.
fn write_mouse_mode(enable: bool) {
    use std::io::Write;
    let out = std::io::stdout();
    let mut out = out.lock();
    for code in [1000u16, 1002, 1003, 1006] {
        let seq = if enable {
            format!("\x1b[?{code}h")
        } else {
            format!("\x1b[?{code}l")
        };
        let _ = out.write_all(seq.as_bytes());
    }
    let _ = out.flush();
}

pub fn size() -> (u16, u16) {
    unsafe {
        let hout = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut info = std::mem::zeroed::<ScreenBufInfo>();
        GetConsoleScreenBufferInfo(hout, &mut info);
        let w = (info.sr_window.right - info.sr_window.left + 1) as u16;
        let h = (info.sr_window.bottom - info.sr_window.top + 1) as u16;
        (w, h)
    }
}

/// Drain irrelevant events from the queue. Returns true when a KEY_DOWN or a
/// MOUSE_EVENT remains available for `read_key`.
fn drain_non_key(hin: Handle) -> bool {
    unsafe {
        loop {
            let mut count = 0u32;
            GetNumberOfConsoleInputEvents(hin, &mut count);
            if count == 0 {
                return false;
            }

            let mut rec = std::mem::zeroed::<InputRecord>();
            let mut peeked = 0u32;
            PeekConsoleInputW(hin, &mut rec, 1, &mut peeked);
            if peeked == 0 {
                return false;
            }

            if is_key_down(&rec) || is_mouse(&rec) {
                return true;
            }

            // Discard useless event (key up, focus, resize, etc.)
            let mut read = 0u32;
            ReadConsoleInputW(hin, &mut rec, 1, &mut read);
        }
    }
}

/// Returns true if a KEY_DOWN is available within the timeout.
pub fn poll(timeout_ms: u64) -> bool {
    unsafe {
        let hin = GetStdHandle(STD_INPUT_HANDLE);
        if drain_non_key(hin) {
            return true;
        }

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let rem = (deadline - now).as_millis().min(50) as Dword;

            if WaitForSingleObject(hin, rem) == WAIT_OBJECT_0 {
                if drain_non_key(hin) {
                    return true;
                }
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
            if read == 0 {
                continue;
            }

            if is_mouse(&rec) {
                return decode_mouse(&rec);
            }
            if !is_key_down(&rec) {
                continue;
            }

            let vk = ke_vk(&rec.event);
            let ch = ke_char(&rec.event);
            let ctrl = ke_ctrl(&rec.event) & (LEFT_CTRL | RIGHT_CTRL) != 0;
            let alt = ke_ctrl(&rec.event) & (LEFT_ALT | RIGHT_ALT) != 0;
            let shift = GetKeyState(VK_SHIFT) as u16 & 0x8000 != 0;

            if ctrl && vk == 0x31 {
                return Key::Ctrl1;
            }
            if ctrl && vk == 0x32 {
                return Key::Ctrl2;
            }
            if ctrl && vk == 0x33 {
                return Key::Ctrl3;
            }
            if ctrl && vk == 0x34 {
                return Key::Ctrl4;
            }
            if ctrl && vk == 0x61 {
                return Key::Ctrl1;
            }
            if ctrl && vk == 0x62 {
                return Key::Ctrl2;
            }
            if ctrl && vk == 0x63 {
                return Key::Ctrl3;
            }
            if ctrl && vk == 0x64 {
                return Key::Ctrl4;
            }
            if alt && vk == 0x26 {
                return Key::AltUp;
            }
            if alt && vk == 0x28 {
                return Key::AltDown;
            }
            if alt && vk == 0x25 {
                return Key::AltLeft;
            }
            if alt && vk == 0x27 {
                return Key::AltRight;
            }
            if ctrl && vk == 0x2E {
                return Key::CtrlDelete;
            }
            if ch == 0x03 || (ctrl && vk == 0x43) {
                return Key::CtrlC;
            }
            if ctrl && vk == 0x44 {
                return Key::CtrlD;
            }
            if ctrl && vk == 0x45 {
                return Key::CtrlE;
            }
            if ctrl && vk == 0x46 {
                return Key::CtrlF;
            }
            if ctrl && vk == 0x48 {
                return Key::CtrlH;
            }
            if ctrl && vk == 0x4A {
                return Key::CtrlJ;
            }
            if ctrl && vk == 0x4B {
                return Key::CtrlK;
            }
            if ctrl && vk == 0x4C {
                return Key::CtrlL;
            }
            if ctrl && vk == 0x4E {
                return Key::CtrlN;
            }
            if ctrl && vk == 0x50 {
                return Key::CtrlP;
            }
            if ctrl && vk == 0x51 {
                return Key::CtrlQ;
            }
            if ctrl && vk == 0x56 {
                return Key::CtrlV;
            }
            if ctrl && vk == 0x57 {
                return Key::CtrlW;
            }
            if ctrl && vk == 0x58 {
                return Key::CtrlX;
            }
            if ctrl && vk == 0x5A {
                return Key::CtrlZ;
            }
            if ctrl && vk == 0x54 {
                return Key::CtrlT;
            }
            if alt && vk == 0x48 {
                return Key::AltH;
            }
            if alt && vk == 0x52 {
                return Key::AltR;
            }
            if alt && vk == 0x56 {
                return Key::AltV;
            }
            if alt && vk == 0x4D {
                return Key::AltM;
            }

            // Ctrl+Enter uses GetKeyState (real-time state) because
            // dwControlKeyState may not report Ctrl correctly in this context.
            if vk == 0x0D {
                let ctrl_held = ctrl || (GetKeyState(0x11) as u16 & 0x8000 != 0);
                if ctrl_held {
                    return Key::CtrlEnter;
                }
                return Key::Enter;
            }

            match vk {
                0x08 => return Key::Backspace,
                0x09 => return Key::Tab,
                0x2E => return Key::Delete,
                0x1B => return Key::Escape,
                0x21 => return Key::PageUp,
                0x22 => return Key::PageDown,
                0x23 => return Key::End,
                0x24 => return Key::Home,
                0x70 => return Key::F1,
                0x26 => return if shift { Key::ShiftUp } else { Key::Up },
                0x28 => return if shift { Key::ShiftDown } else { Key::Down },
                0x25 => return if shift { Key::ShiftLeft } else { Key::Left },
                0x27 => return if shift { Key::ShiftRight } else { Key::Right },
                _ => {}
            }

            // VT hosts (e.g. Windows Terminal via ConPTY) deliver key
            // sequences as character records instead of VK records: reassemble
            // ESC O P (F1), ESC[11~ (F1), arrows, etc. from those characters.
            let pending = VT_PENDING.swap(VT_PENDING_NONE, std::sync::atomic::Ordering::Relaxed);
            if pending != VT_PENDING_NONE
                && let Some(key) = vt_feed(pending)
            {
                return key;
            }
            if let Some(key) = vt_feed(ch) {
                return key;
            }
        }
    }
}

/// VT input reassembly state: characters that arrive as separate
/// `KEY_EVENT_RECORD`s while forming an ESC sequence (ConPTY hosts).
/// `VT_PENDING` holds a character that followed a lone ESC and is fed
/// back on the next read (0xFFFF = empty).
static VT_SEQ: [std::sync::atomic::AtomicU16; 24] =
    [const { std::sync::atomic::AtomicU16::new(0) }; 24];
static VT_SEQ_LEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static VT_PENDING: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0xFFFF);
const VT_PENDING_NONE: u16 = 0xFFFF;

// Character constants for the VT pattern matches below.
const K_ESC: u16 = 0x1B;
const K_LBRACKET: u16 = 0x5B; // '['
const K_O: u16 = 0x4F; // 'O'
const K_TILDE: u16 = 0x7E; // '~'
const K_A: u16 = 0x41; // 'A'
const K_B: u16 = 0x42; // 'B'
const K_C: u16 = 0x43; // 'C'
const K_D: u16 = 0x44; // 'D'
const K_H: u16 = 0x48; // 'H'
const K_F: u16 = 0x46; // 'F'
const K_P: u16 = 0x50; // 'P'
const K_ZERO: u16 = 0x30; // '0'
const K_NINE: u16 = 0x39; // '9'
const K_SEMICOLON: u16 = 0x3B; // ';'

/// Feed one character from the input stream into the VT key reassembler.
/// Returns the decoded `Key` when the character completes a sequence (or is
/// plain text); None while a sequence is still being accumulated.
fn vt_feed(ch: u16) -> Option<Key> {
    let len = VT_SEQ_LEN.load(std::sync::atomic::Ordering::Relaxed);
    if len == 0 {
        if ch == K_ESC {
            VT_SEQ[0].store(ch, std::sync::atomic::Ordering::Relaxed);
            VT_SEQ_LEN.store(1, std::sync::atomic::Ordering::Relaxed);
            return None;
        }
        VT_PENDING.store(VT_PENDING_NONE, std::sync::atomic::Ordering::Relaxed);
        return match ch {
            0x08 => Some(Key::Backspace),
            0x0D => Some(Key::Enter),
            0x09 => Some(Key::Tab),
            0x03 => Some(Key::CtrlC),
            0x04 => Some(Key::CtrlD),
            0x0C => Some(Key::CtrlL),
            0x1A => Some(Key::CtrlZ),
            c if c < 0x20 => None,
            c => char::from_u32(c as u32).map(Key::Char),
        };
    }

    if VT_SEQ[0].load(std::sync::atomic::Ordering::Relaxed) != K_ESC {
        VT_SEQ_LEN.store(0, std::sync::atomic::Ordering::Relaxed);
        return None;
    }

    // ESC followed by a printable, but not a sequence intro: the ESC was
    // a bare Escape; the character is fed back on the next read.
    if len == 1 && ch != K_LBRACKET && ch != K_O {
        VT_SEQ_LEN.store(0, std::sync::atomic::Ordering::Relaxed);
        VT_PENDING.store(ch, std::sync::atomic::Ordering::Relaxed);
        return Some(Key::Escape);
    }

    // ESC '[' / ESC 'O': append the intro and wait for the rest.
    if len == 1 {
        VT_SEQ[1].store(ch, std::sync::atomic::Ordering::Relaxed);
        VT_SEQ_LEN.store(2, std::sync::atomic::Ordering::Relaxed);
        return None;
    }

    // ESC O <final>: SS3 function/navigation keys.
    if len == 2 && VT_SEQ[1].load(std::sync::atomic::Ordering::Relaxed) == K_O {
        VT_SEQ_LEN.store(0, std::sync::atomic::Ordering::Relaxed);
        return match ch {
            K_P => Some(Key::F1),
            K_A => Some(Key::Up),
            K_B => Some(Key::Down),
            K_C => Some(Key::Right),
            K_D => Some(Key::Left),
            _ => {
                VT_PENDING.store(ch, std::sync::atomic::Ordering::Relaxed);
                Some(Key::Escape)
            }
        };
    }

    // ESC [ <params> <final>.
    if VT_SEQ[1].load(std::sync::atomic::Ordering::Relaxed) == K_LBRACKET {
        let is_final = (0x40..=0x7E).contains(&ch);
        if len == 2 && !is_final && !((K_ZERO..=K_NINE).contains(&ch) || ch == K_SEMICOLON) {
            VT_SEQ_LEN.store(0, std::sync::atomic::Ordering::Relaxed);
            VT_PENDING.store(ch, std::sync::atomic::Ordering::Relaxed);
            return Some(Key::Escape);
        }
        if len < VT_SEQ.len() {
            VT_SEQ[len].store(ch, std::sync::atomic::Ordering::Relaxed);
            VT_SEQ_LEN.store(len + 1, std::sync::atomic::Ordering::Relaxed);
        }
        if !is_final {
            return None; // still accumulating parameters
        }
        let key = vt_finish_csi(len + 1);
        VT_SEQ_LEN.store(0, std::sync::atomic::Ordering::Relaxed);
        return key;
    }

    VT_SEQ_LEN.store(0, std::sync::atomic::Ordering::Relaxed);
    None
}

/// Decode a completed `ESC [ ...` sequence (params in `VT_SEQ[2..]`).
fn vt_finish_csi(len: usize) -> Option<Key> {
    if len < 3 {
        return None;
    }
    let final_byte = VT_SEQ[len - 1].load(std::sync::atomic::Ordering::Relaxed);
    let mut params = String::new();
    for entry in VT_SEQ.iter().take(len - 1).skip(2) {
        if let Some(c) = char::from_u32(entry.load(std::sync::atomic::Ordering::Relaxed) as u32) {
            params.push(c);
        }
    }
    match final_byte {
        K_TILDE => match params.as_str() {
            "11" => Some(Key::F1),
            "3" => Some(Key::Delete),
            "3;5" => Some(Key::CtrlDelete),
            "5" => Some(Key::PageUp),
            "6" => Some(Key::PageDown),
            _ => None,
        },
        K_A => Some(arrow_with_mods(&params, Key::Up, Key::ShiftUp, Key::AltUp)),
        K_B => Some(arrow_with_mods(
            &params,
            Key::Down,
            Key::ShiftDown,
            Key::AltDown,
        )),
        K_C => Some(arrow_with_mods(
            &params,
            Key::Right,
            Key::ShiftRight,
            Key::AltRight,
        )),
        K_D => Some(arrow_with_mods(
            &params,
            Key::Left,
            Key::ShiftLeft,
            Key::AltLeft,
        )),
        K_H => Some(Key::Home),
        K_F => Some(Key::End),
        _ => None,
    }
}

/// Pick the arrow key variant from the CSI modifier parameter: plain,
/// Shift ("1;2") or Alt ("1;3").
fn arrow_with_mods(params: &str, plain: Key, shift: Key, alt: Key) -> Key {
    match params {
        "1;2" => shift,
        "1;3" => alt,
        _ => plain,
    }
}

pub fn held_arrow_keys() -> super::HeldArrowKeys {
    unsafe {
        super::HeldArrowKeys {
            up: GetKeyState(0x26) as u16 & 0x8000 != 0,
            down: GetKeyState(0x28) as u16 & 0x8000 != 0,
            left: GetKeyState(0x25) as u16 & 0x8000 != 0,
            right: GetKeyState(0x27) as u16 & 0x8000 != 0,
        }
    }
}

// ── Clipboard (Win32 FFI) ─────────────────────────────────────────────────────

/// Write `text` (Unicode) to the system clipboard. Returns true on success.
pub fn clipboard_set(text: &str) -> bool {
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        if EmptyClipboard() == 0 {
            let _ = CloseClipboard();
            return false;
        }

        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let size = wide.len() * 2;
        let h = GlobalAlloc(GMEM_MOVEABLE, size);
        if h.is_null() {
            let _ = CloseClipboard();
            return false;
        }
        let p = GlobalLock(h);
        if p.is_null() {
            let _ = GlobalFree(h);
            let _ = CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p as *mut u16, wide.len());
        let _ = GlobalUnlock(h);

        if SetClipboardData(CF_UNICODETEXT, h).is_null() {
            let _ = GlobalFree(h);
            let _ = CloseClipboard();
            return false;
        }
        let _ = CloseClipboard();
        true
    }
}

/// Read the system clipboard as a String (Unicode). None when unavailable.
pub fn clipboard_get() -> Option<String> {
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let h = GetClipboardData(CF_UNICODETEXT);
        if h.is_null() {
            let _ = CloseClipboard();
            return None;
        }
        let p = GlobalLock(h);
        if p.is_null() {
            let _ = CloseClipboard();
            return None;
        }
        let size = (GlobalSize(h) / 2).min(1 << 20) as usize;
        let mut v: Vec<u16> = vec![0u16; size];
        std::ptr::copy_nonoverlapping(p as *const u16, v.as_mut_ptr(), size);
        let _ = GlobalUnlock(h);
        let _ = CloseClipboard();
        while v.last() == Some(&0) {
            v.pop();
        }
        Some(String::from_utf16_lossy(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::{MouseAction, MouseButton};

    fn mouse_record(x: i16, y: i16, button: u32, ctrl: u32, flags: u32) -> InputRecord {
        let mut event = [0u8; 16];
        event[0..2].copy_from_slice(&x.to_ne_bytes());
        event[2..4].copy_from_slice(&y.to_ne_bytes());
        event[4..8].copy_from_slice(&button.to_ne_bytes());
        event[8..12].copy_from_slice(&ctrl.to_ne_bytes());
        event[12..16].copy_from_slice(&flags.to_ne_bytes());
        InputRecord {
            event_type: MOUSE_EVENT_TYPE,
            _pad: 0,
            event,
        }
    }

    /// A KEY_EVENT_RECORD carrying one character (ConPTY-style VT input).
    fn char_record(ch: u16) -> InputRecord {
        let mut event = [0u8; 16];
        event[0..4].copy_from_slice(&1i32.to_ne_bytes()); // bKeyDown = 1
        event[6..8].copy_from_slice(&0u16.to_ne_bytes()); // wVK = 0
        event[10..12].copy_from_slice(&ch.to_ne_bytes()); // uChar
        event[12..16].copy_from_slice(&0u32.to_ne_bytes()); // dwControlKeyState
        InputRecord {
            event_type: KEY_EVENT_TYPE,
            _pad: 0,
            event,
        }
    }

    fn vt_seqs() -> Vec<(Vec<u16>, Key)> {
        vec![
            (vec![0x1B, b'O' as u16, b'P' as u16], Key::F1),
            (
                vec![0x1B, b'[' as u16, b'1' as u16, b'1' as u16, b'~' as u16],
                Key::F1,
            ),
            (vec![0x1B, b'[' as u16, b'A' as u16], Key::Up),
            (
                vec![
                    0x1B,
                    b'[' as u16,
                    b'1' as u16,
                    b';' as u16,
                    b'2' as u16,
                    b'A' as u16,
                ],
                Key::ShiftUp,
            ),
            (
                vec![
                    0x1B,
                    b'[' as u16,
                    b'1' as u16,
                    b';' as u16,
                    b'3' as u16,
                    b'D' as u16,
                ],
                Key::AltLeft,
            ),
            (
                vec![0x1B, b'[' as u16, b'5' as u16, b'~' as u16],
                Key::PageUp,
            ),
            (vec![0x1B, b'[' as u16, b'F' as u16], Key::End),
            (vec![0x1B, b'O' as u16, b'A' as u16], Key::Up),
        ]
    }

    #[test]
    fn vt_feed_reassembles_sequences() {
        for (seq, expected) in vt_seqs() {
            VT_SEQ_LEN.store(0, std::sync::atomic::Ordering::Relaxed);
            VT_PENDING.store(VT_PENDING_NONE, std::sync::atomic::Ordering::Relaxed);
            for (i, &ch) in seq.iter().enumerate() {
                let key = vt_feed(ch);
                if i + 1 == seq.len() {
                    assert_eq!(key, Some(expected), "seq {seq:?} must end on {expected:?}");
                } else {
                    assert!(
                        key.is_none(),
                        "seq {seq:?} must not complete early ({key:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn vt_feed_plain_char_and_lone_escape() {
        VT_SEQ_LEN.store(0, std::sync::atomic::Ordering::Relaxed);
        VT_PENDING.store(VT_PENDING_NONE, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(vt_feed(b'a' as u16), Some(Key::Char('a')));
        assert_eq!(vt_feed(0x08), Some(Key::Backspace));
        // A lone ESC is delivered as Escape; the next char is fed back.
        assert_eq!(vt_feed(0x1B), None);
        assert_eq!(vt_feed(b'x' as u16), Some(Key::Escape));
        assert_eq!(
            VT_PENDING.load(std::sync::atomic::Ordering::Relaxed),
            b'x' as u16
        );
        VT_PENDING.store(VT_PENDING_NONE, std::sync::atomic::Ordering::Relaxed);
        // And throws away unknown control bytes.
        assert_eq!(vt_feed(0x01), None);
    }

    #[test]
    fn decode_mouse_left_press() {
        let rec = mouse_record(5, 3, FROM_LEFT_1ST_BUTTON_PRESSED, 0, 0);
        let Key::Mouse(ev) = decode_mouse(&rec) else {
            panic!("expected mouse")
        };
        assert_eq!(ev.x, 6); // 1-based
        assert_eq!(ev.y, 4);
        assert_eq!(ev.kind, MouseAction::Press);
        assert_eq!(ev.button, MouseButton::Left);
    }

    #[test]
    fn decode_mouse_release_and_wheel() {
        // Move with no button -> release-like Move event.
        let rec = mouse_record(2, 2, 0, 0, MOUSE_MOVED);
        let Key::Mouse(ev) = decode_mouse(&rec) else {
            panic!()
        };
        assert_eq!(ev.kind, MouseAction::Move);

        // Wheel up: MOUSE_WHEELED with a positive signed delta in the high word.
        let rec = mouse_record(2, 2, 1 << 16, 0, MOUSE_WHEELED);
        let Key::Mouse(ev) = decode_mouse(&rec) else {
            panic!()
        };
        assert_eq!(ev.kind, MouseAction::Press);
        assert_eq!(ev.button, MouseButton::WheelUp);

        // No button, no flags (a button just released).
        let rec = mouse_record(2, 2, 0, 0, 0);
        let Key::Mouse(ev) = decode_mouse(&rec) else {
            panic!()
        };
        assert_eq!(ev.kind, MouseAction::Release);
    }

    #[test]
    fn decode_mouse_ignores_key_records() {
        let mut event = [0u8; 16];
        event[0..4].copy_from_slice(&1i32.to_ne_bytes()); // key down
        event[10..12].copy_from_slice(&('a' as u16).to_ne_bytes());
        let rec = InputRecord {
            event_type: KEY_EVENT_TYPE,
            _pad: 0,
            event,
        };
        assert!(!is_mouse(&rec));
        assert!(is_key_down(&rec));
    }

    #[test]
    fn read_key_decodes_injected_vt_f1_records() {
        // Skip when the test process has no real console input buffer.
        unsafe {
            let hin = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode: Dword = 0;
            if GetConsoleMode(hin, &mut mode) == 0 {
                return;
            }
            // Inject ESC O P as three ConPTY-style character records and read
            // them back through the real `read_key` path.
            let recs = [
                char_record(0x1B),
                char_record(b'O' as u16),
                char_record(b'P' as u16),
            ];
            let mut written: Dword = 0;
            if WriteConsoleInputW(hin, recs.as_ptr(), 3, &mut written) == 0 || written != 3 {
                return;
            }
        }
        // read_key returns only when a full sequence decodes: F1.
        assert_eq!(read_key(), Key::F1, "ESC O P records must decode to F1");
    }

    #[test]
    fn read_key_decodes_injected_ctrl_numpad_records() {
        // Skip when the test process has no real console input buffer.
        unsafe {
            let hin = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode: Dword = 0;
            if GetConsoleMode(hin, &mut mode) == 0 {
                return;
            }
            // Ctrl+Numpad1..4 arrive as KEY_EVENT_RECORDs with VK_NUMPADn
            // (0x61..0x64) and the Ctrl modifier bit set.
            let recs = [vk_record(0x61, LEFT_CTRL), vk_record(0x64, LEFT_CTRL)];
            let mut written: Dword = 0;
            if WriteConsoleInputW(hin, recs.as_ptr(), 2, &mut written) == 0 || written != 2 {
                return;
            }
        }
        assert_eq!(read_key(), Key::Ctrl1, "Ctrl+Numpad1 must decode to Ctrl1");
        assert_eq!(read_key(), Key::Ctrl4, "Ctrl+Numpad4 must decode to Ctrl4");
    }

    /// A KEY_EVENT_RECORD carrying a virtual key code with modifier state.
    fn vk_record(vk: u16, ctrl: u32) -> InputRecord {
        let mut event = [0u8; 16];
        event[0..4].copy_from_slice(&1i32.to_ne_bytes()); // bKeyDown = 1
        event[6..8].copy_from_slice(&vk.to_ne_bytes()); // wVK
        event[10..12].copy_from_slice(&('1' as u16).to_ne_bytes()); // uChar
        event[12..16].copy_from_slice(&ctrl.to_ne_bytes()); // dwControlKeyState
        InputRecord {
            event_type: KEY_EVENT_TYPE,
            _pad: 0,
            event,
        }
    }
}
