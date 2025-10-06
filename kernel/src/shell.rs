use crate::{serial, vga};
use crate::keyboard;
use crate::ramfs;
use crate::pmm;

static mut LINE: [u8; 256] = [0; 256];
static mut LEN: usize = 0;

pub fn step() {
    while let Some(c) = keyboard::poll_char() {
        match c {
            '\n' => { execute_line(); clear_line(); prompt(); }
            '\x08' => { if unsafe { LEN } > 0 { unsafe { LEN -= 1; } } }
            ch if (ch as u32) >= 32 && (ch as u32) < 127 => {
                if unsafe { LEN } < unsafe { LINE.len() } { unsafe { LINE[LEN] = ch as u8; LEN += 1; } }
            }
            _ => {}
        }
    }
}

fn prompt() {
    serial::write_str("$ ");
    vga::write_str("$ ");
}

fn clear_line() { unsafe { LEN = 0; } }

fn execute_line() {
    let (cmd, arg) = unsafe {
        let s = core::str::from_utf8_unchecked(&LINE[..LEN]);
        split1(s)
    };
    match cmd {
        "" => {}
        "help" => {
            writeln("Commands: help, ls, cat <path>, mem, ai, pci");
        }
        "ls" => {
            ramfs::for_each(|e| {
                if let Ok(name) = core::str::from_utf8(e.name) { writeln(name); }
            });
        }
        "cat" => {
            if arg.is_empty() { writeln("usage: cat <path>"); return; }
            if let Some((ptr, size)) = ramfs::find(arg) {
                unsafe {
                    let bytes = core::slice::from_raw_parts(ptr, size.min(1024));
                    if let Ok(s) = core::str::from_utf8(bytes) { write_str(s); }
                    else { writeln("(binary)" ); }
                }
            } else { writeln("not found"); }
        }
        "mem" => {
            let kib = pmm::free_kib();
            writeln_num("free_kib=", kib);
        }
        _ => writeln("unknown command"),
    }
}

fn split1(s: &str) -> (&str, &str) {
    let s = s.trim();
    if s.is_empty() { return ("", ""); }
    if let Some(sp) = s.find(' ') { (&s[..sp], s[sp+1..].trim()) } else { (s, "") }
}

fn write_str(s: &str) {
    serial::write_str(s);
    vga::write_str(s);
}

fn writeln(s: &str) { write_str(s); write_str("\r\n"); }

fn writeln_num(prefix: &str, n: u64) {
    serial::write_fmt(format_args!("{}{}\r\n", prefix, n));
    vga::write_line(prefix); // simplistic; number not printed on VGA here
}

pub fn start() {
    prompt();
}

