use core::sync::atomic::{AtomicBool, Ordering};

use crate::{serial, vga};

static CTRL_HELD: AtomicBool = AtomicBool::new(false);
static SHIFT_HELD: AtomicBool = AtomicBool::new(false);

// US QWERTY set-1 scancode to ASCII maps (partial but practical)
// Index by scancode without the release bit (0x80 cleared)
const MAP_NORMAL: [Option<char>; 0x3A] = {
    let mut m = [None; 0x3A];
    m[0x02] = Some('1');
    m[0x03] = Some('2');
    m[0x04] = Some('3');
    m[0x05] = Some('4');
    m[0x06] = Some('5');
    m[0x07] = Some('6');
    m[0x08] = Some('7');
    m[0x09] = Some('8');
    m[0x0A] = Some('9');
    m[0x0B] = Some('0');
    m[0x0C] = Some('-');
    m[0x0D] = Some('=');
    // 0x0E backspace
    // 0x0F tab
    m[0x10] = Some('q');
    m[0x11] = Some('w');
    m[0x12] = Some('e');
    m[0x13] = Some('r');
    m[0x14] = Some('t');
    m[0x15] = Some('y');
    m[0x16] = Some('u');
    m[0x17] = Some('i');
    m[0x18] = Some('o');
    m[0x19] = Some('p');
    m[0x1A] = Some('[');
    m[0x1B] = Some(']');
    // 0x1C enter
    // 0x1D ctrl
    m[0x1E] = Some('a');
    m[0x1F] = Some('s');
    m[0x20] = Some('d');
    m[0x21] = Some('f');
    m[0x22] = Some('g');
    m[0x23] = Some('h');
    m[0x24] = Some('j');
    m[0x25] = Some('k');
    m[0x26] = Some('l');
    m[0x27] = Some(';');
    m[0x28] = Some('\'');
    m[0x29] = Some('`');
    // 0x2A left shift
    m[0x2B] = Some('\\');
    m[0x2C] = Some('z');
    m[0x2D] = Some('x');
    m[0x2E] = Some('c');
    m[0x2F] = Some('v');
    m[0x30] = Some('b');
    m[0x31] = Some('n');
    m[0x32] = Some('m');
    m[0x33] = Some(',');
    m[0x34] = Some('.');
    m[0x35] = Some('/');
    // 0x36 right shift
    // 0x37 keypad * (ignored)
    m[0x39] = Some(' ');
    m
};

const MAP_SHIFT: [Option<char>; 0x3A] = {
    let mut m = [None; 0x3A];
    m[0x02] = Some('!');
    m[0x03] = Some('@');
    m[0x04] = Some('#');
    m[0x05] = Some('$');
    m[0x06] = Some('%');
    m[0x07] = Some('^');
    m[0x08] = Some('&');
    m[0x09] = Some('*');
    m[0x0A] = Some('(');
    m[0x0B] = Some(')');
    m[0x0C] = Some('_');
    m[0x0D] = Some('+');
    m[0x10] = Some('Q');
    m[0x11] = Some('W');
    m[0x12] = Some('E');
    m[0x13] = Some('R');
    m[0x14] = Some('T');
    m[0x15] = Some('Y');
    m[0x16] = Some('U');
    m[0x17] = Some('I');
    m[0x18] = Some('O');
    m[0x19] = Some('P');
    m[0x1A] = Some('{');
    m[0x1B] = Some('}');
    m[0x1E] = Some('A');
    m[0x1F] = Some('S');
    m[0x20] = Some('D');
    m[0x21] = Some('F');
    m[0x22] = Some('G');
    m[0x23] = Some('H');
    m[0x24] = Some('J');
    m[0x25] = Some('K');
    m[0x26] = Some('L');
    m[0x27] = Some(':');
    m[0x28] = Some('"');
    m[0x29] = Some('~');
    m[0x2B] = Some('|');
    m[0x2C] = Some('Z');
    m[0x2D] = Some('X');
    m[0x2E] = Some('C');
    m[0x2F] = Some('V');
    m[0x30] = Some('B');
    m[0x31] = Some('N');
    m[0x32] = Some('M');
    m[0x33] = Some('<');
    m[0x34] = Some('>');
    m[0x35] = Some('?');
    m[0x39] = Some(' ');
    m
};

/// Handles a raw set-1 scancode; returns the combo description when a shutdown should be triggered.
pub fn handle_scancode(scancode: u8) -> Option<&'static str> {
    // Ignore extended prefix bytes for now.
    if scancode == 0xE0 {
        return None;
    }

    let is_release = scancode & 0x80 != 0;
    let code = scancode & 0x7F;

    match code {
        // Left/Right Shift
        0x2A | 0x36 => {
            SHIFT_HELD.store(!is_release, Ordering::Relaxed);
            None
        }
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
        0x0E => {
            // Backspace
            if !is_release {
                vga::backspace();
            }
            None
        }
        0x0F => {
            // Tab -> 4 spaces for simplicity
            if !is_release {
                vga::write_str("    ");
            }
            None
        }
        0x1C => {
            // Enter
            if !is_release {
                vga::put_char('\n');
            }
            None
        }
        _ => {
            if !is_release {
                let shifted = SHIFT_HELD.load(Ordering::Relaxed);
                let ch = if shifted {
                    MAP_SHIFT.get(code as usize).and_then(|c| *c)
                } else {
                    MAP_NORMAL.get(code as usize).and_then(|c| *c)
                };
                if let Some(c) = ch {
                    vga::put_char(c);
                }
            }
            None
        }
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
