// Input uses ReadConsoleInputW instead of ReadFile+VT to avoid blocking:
// WaitForSingleObject signals on any console event (focus, etc.), while
// ReadFile would block waiting for VT bytes that never arrive. With
// ReadConsoleInputW records are read directly and non-keyboard events are
// discarded.

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
const LEFT_ALT:        Dword = 0x0002;
const RIGHT_ALT:       Dword = 0x0001;
const VK_SHIFT:        i32   = 0x10;
const GMEM_MOVEABLE:   Dword = 0x0002;
const CF_UNICODETEXT:  Dword = 13;

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

static mut ORIG_IN_MODE:  Dword = 0;
static mut ORIG_OUT_MODE: Dword = 0;

// Helpers to read KEY_EVENT_RECORD fields out of event: [u8; 16].
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

        // No VT input: records are read directly via ReadConsoleInputW.
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

/// Drain non-KEY_DOWN events from the queue. Returns true if a KEY_DOWN
/// remains available.
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

            // Discard useless event (key up, mouse, focus, etc.)
            let mut read = 0u32;
            ReadConsoleInputW(hin, &mut rec, 1, &mut read);
        }
    }
}

/// Returns true if a KEY_DOWN is available within the timeout.
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
            let alt  = ke_ctrl(&rec.event) & (LEFT_ALT | RIGHT_ALT) != 0;
            let shift = GetKeyState(VK_SHIFT) as u16 & 0x8000 != 0;

            if ctrl && vk == 0x31 { return Key::Ctrl1; }
            if ctrl && vk == 0x32 { return Key::Ctrl2; }
            if ctrl && vk == 0x33 { return Key::Ctrl3; }
            if ctrl && vk == 0x34 { return Key::Ctrl4; }
            if alt && vk == 0x26 { return Key::AltUp; }
            if alt && vk == 0x28 { return Key::AltDown; }
            if alt && vk == 0x25 { return Key::AltLeft; }
            if alt && vk == 0x27 { return Key::AltRight; }
            if ctrl && vk == 0x2E { return Key::CtrlDelete; }
            if ch == 0x03 || (ctrl && vk == 0x43) { return Key::CtrlC; }
            if ctrl && vk == 0x44 { return Key::CtrlD; }
            if ctrl && vk == 0x45 { return Key::CtrlE; }
            if ctrl && vk == 0x46 { return Key::CtrlF; }
            if ctrl && vk == 0x48 { return Key::CtrlH; }
            if ctrl && vk == 0x4A { return Key::CtrlJ; }
            if ctrl && vk == 0x4B { return Key::CtrlK; }
            if ctrl && vk == 0x4C { return Key::CtrlL; }
            if ctrl && vk == 0x4E { return Key::CtrlN; }
            if ctrl && vk == 0x50 { return Key::CtrlP; }
            if ctrl && vk == 0x51 { return Key::CtrlQ; }
            if ctrl && vk == 0x56 { return Key::CtrlV; }
            if ctrl && vk == 0x57 { return Key::CtrlW; }
            if ctrl && vk == 0x58 { return Key::CtrlX; }
            if ctrl && vk == 0x5A { return Key::CtrlZ; }
            if ctrl && vk == 0x54 { return Key::CtrlT; }
            if alt && vk == 0x48 { return Key::AltH; }
            if alt && vk == 0x52 { return Key::AltR; }
            if alt && vk == 0x56 { return Key::AltV; }

            // Ctrl+Enter uses GetKeyState (real-time state) because
            // dwControlKeyState may not report Ctrl correctly in this context.
            if vk == 0x0D {
                let ctrl_held = ctrl || (GetKeyState(0x11) as u16 & 0x8000 != 0);
                if ctrl_held { return Key::CtrlEnter; }
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
                0x26 => return if shift { Key::ShiftUp } else { Key::Up },
                0x28 => return if shift { Key::ShiftDown } else { Key::Down },
                0x25 => return if shift { Key::ShiftLeft } else { Key::Left },
                0x27 => return if shift { Key::ShiftRight } else { Key::Right },
                _ => {}
            }

            if let Some(c) = char::from_u32(ch as u32) {
                if !c.is_control() { return Key::Char(c); }
            }
        }
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
