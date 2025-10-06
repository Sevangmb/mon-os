use core::sync::atomic::{AtomicBool, Ordering};

use crate::serial;

static CTRL_HELD: AtomicBool = AtomicBool::new(false);

/// Handles a raw set-1 scancode; returns the combo description when a shutdown should be triggered.
pub fn handle_scancode(scancode: u8) -> Option<&'static str> {
    // Ignore extended prefix bytes for now.
    if scancode == 0xE0 {
        return None;
    }

    let is_release = scancode & 0x80 != 0;
    let code = scancode & 0x7F;

    match code {
        0x1D => {
            CTRL_HELD.store(!is_release, Ordering::Relaxed);
            None
        }
        0x2D => {
            if !is_release && CTRL_HELD.load(Ordering::Relaxed) {
                Some("Ctrl+X")
            } else {
                None
            }
        }
        0x2E => {
            if !is_release && CTRL_HELD.load(Ordering::Relaxed) {
                Some("Ctrl+C")
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn shutdown_via_keyboard(combo: &str) -> ! {
    serial::write_fmt(format_args!("[KEYBOARD] {}\r\n", combo));
    crate::exit_qemu(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::Ordering;

    #[test]
    fn ctrl_x_triggers_shutdown() {
        CTRL_HELD.store(false, Ordering::Relaxed);

        assert_eq!(handle_scancode(0x1D), None);
        assert_eq!(handle_scancode(0x2D), Some("Ctrl+X"));
        handle_scancode(0x9D);
        CTRL_HELD.store(false, Ordering::Relaxed);
    }

    #[test]
    fn ctrl_c_triggers_shutdown() {
        CTRL_HELD.store(false, Ordering::Relaxed);

        assert_eq!(handle_scancode(0x1D), None);
        assert_eq!(handle_scancode(0x2E), Some("Ctrl+C"));
        handle_scancode(0x9D);
        CTRL_HELD.store(false, Ordering::Relaxed);
    }

    #[test]
    fn release_clears_ctrl_state() {
        CTRL_HELD.store(false, Ordering::Relaxed);

        handle_scancode(0x1D);
        handle_scancode(0x9D);
        assert_eq!(handle_scancode(0x2D), None);
    }
}
