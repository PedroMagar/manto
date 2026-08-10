// Manto: a terminal-driven desktop environment.
//
// Entry point only: terminal setup, the main loop, and teardown. All state
// and event handling live in `app::Desktop`; drawing lives in `ui`.

mod app;
mod cmd;
mod config;
mod input;
mod json;
mod menu;
mod os;
mod session;
mod terminal_backend;
mod terminal_emulator;
mod ui;
mod wm;

use std::io::Write;
use std::time::Duration;

use app::Desktop;
use os::{Clock, Writer};
use ui::ansi;

fn main() {
    let mut out = Writer::new();

    os::enable_raw_mode();
    ansi::enter_alt_screen(&mut out);
    ansi::hide_cursor(&mut out);
    out.flush().unwrap();

    let mut desktop = Desktop::new();
    desktop.draw(&mut out);

    let mut last_check = Clock::now();

    loop {
        if os::poll(50) {
            if desktop.step_input() {
                desktop.draw(&mut out);
            }
            if desktop.quit {
                break;
            }
        }

        if desktop.tick() {
            desktop.draw(&mut out);
        }

        if last_check.elapsed() >= Duration::from_secs(1) {
            if desktop.on_second_tick() {
                desktop.draw(&mut out);
            }
            last_check = Clock::now();
        }
    }

    ansi::leave_alt_screen(&mut out);
    ansi::show_cursor(&mut out);
    out.flush().unwrap();
    os::disable_raw_mode();
}
