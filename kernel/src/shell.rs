use crate::{serial, vga};
use crate::keyboard;
use crate::ramfs;
use crate::pmm;
use crate::idt;
use crate::apply_action;

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
            writeln("Commands: help, ls, cat <path>, hexdump <path> [len], mem, uptime, ai, pci, reboot, sleep <ms>, yield");
        }
        "ls" => {
            ramfs::for_each(|e| {
                if let Ok(name) = core::str::from_utf8(e.name) {
                    serial::write_fmt(format_args!("{} {}\r\n", name, e.size));
                    vga::write_str(name);
                    vga::put_char(' ');
                    print_num(e.size as u64);
                    vga::put_char('\n');
                }
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
        "hexdump" => {
            if arg.is_empty() { writeln("usage: hexdump <path> [len]"); return; }
            let (path, rest) = split1(arg);
            let mut dump_len: usize = 256;
            if !rest.is_empty() { if let Some(v) = parse_u64(rest) { dump_len = v as usize; } }
            if let Some((ptr, size)) = ramfs::find(path) {
                let n = core::cmp::min(size, dump_len);
                unsafe {
                    let bytes = core::slice::from_raw_parts(ptr, n);
                    hex_dump(bytes);
                }
            } else { writeln("not found"); }
        }
        "mem" => {
            let kib = pmm::free_kib();
            writeln_num("free_kib=", kib);
        }
        "uptime" => {
            let t = idt::timer_ticks();
            writeln_num("ticks=", t);
        }
        "ai" => {
            unsafe {
                extern "C" { static mut AI_MODEL_ADDR: *const u8; static mut AI_MODEL_LEN: usize; }
                let addr = AI_MODEL_ADDR as u64;
                let len = AI_MODEL_LEN as u64;
                writeln_num("ai_model_addr=", addr);
                writeln_num("ai_model_len=", len);
            }
            let ready = apply_action::is_system_ready();
            serial::write_fmt(format_args!("system_ready={}\r\n", ready as u8));
            vga::write_line(if ready { "system_ready=1" } else { "system_ready=0" });
            let q = apply_action::get_quantum_us() as u64;
            writeln_num("quantum_us=", q);
        }
        "pci" => {
            crate::log_usb_controllers();
        }
        "reboot" => {
            crate::exit_qemu(0);
        }
        "sleep" => {
            if arg.is_empty() { writeln("usage: sleep <ms>"); return; }
            if let Some(ms) = parse_u64(arg) {
                sleep_ms(ms as u64);
            }
        }
        "yield" => {
            unsafe { core::arch::asm!("hlt"); }
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
    vga::write_str(prefix);
    let mut buf = [0u8; 20];
    let mut x = n;
    if x == 0 { vga::put_char('0'); vga::put_char('\n'); return; }
    let mut i = buf.len();
    while x > 0 { i -= 1; buf[i] = b'0' + (x % 10) as u8; x /= 10; }
    for &b in &buf[i..] { vga::put_char(b as char); }
    vga::put_char('\n');
}

pub fn start() {
    prompt();
}

fn parse_u64(s: &str) -> Option<u64> {
    let mut v: u64 = 0;
    for c in s.bytes() {
        if c < b'0' || c > b'9' { return None; }
        v = v.saturating_mul(10).saturating_add((c - b'0') as u64);
    }
    Some(v)
}

fn print_num(n: u64) {
    let mut buf = [0u8; 20];
    let mut x = n;
    if x == 0 { vga::put_char('0'); return; }
    let mut i = buf.len();
    while x > 0 { i -= 1; buf[i] = b'0' + (x % 10) as u8; x /= 10; }
    for &b in &buf[i..] { vga::put_char(b as char); }
}

fn hex_dump(bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut off = 0usize;
    while off < bytes.len() {
        // offset
        serial::write_fmt(format_args!("{:08x}  ", off));
        for i in 0..16 {
            if off + i < bytes.len() {
                let b = bytes[off + i];
                serial::write_fmt(format_args!("{:02x} ", b));
            } else {
                serial::write_str("   ");
            }
        }
        serial::write_str(" |");
        for i in 0..16 {
            if off + i < bytes.len() {
                let ch = bytes[off + i];
                let c = if ch >= 32 && ch < 127 { ch as char } else { '.' };
                serial::write_fmt(format_args!("{}", c));
            }
        }
        serial::write_str("|\r\n");
        off += 16;
    }
}

// Approximate sleep using timer ticks (assumes ~1 kHz timer)
fn sleep_ms(ms: u64) {
    let start = idt::timer_ticks();
    let target = start.saturating_add(ms);
    while idt::timer_ticks() < target {
        unsafe { core::arch::asm!("hlt"); }
    }
}
