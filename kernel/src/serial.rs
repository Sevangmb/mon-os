use core::fmt::{self, Write};
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};
use spin::Mutex;
use x86_64::instructions::port::Port;

const COM1_BASE: u16 = 0x3F8;

pub fn init() {
    dbg_str("serial: init start\n");
    {
        let mut serial = SERIAL.lock();
        unsafe {
            serial.init();
        }
    }
    INITIALIZED.store(true, Ordering::Release);
    dbg_str("serial: init done\n");
}

pub fn write_str(message: &str) {
    dbg_str("serial: write_str\n");
    if !is_ready() {
        dbg_fmt(format_args!("{}", message));
        return;
    }

    let mut serial = SERIAL.lock();
    let _ = serial.write_str(message);
}

pub fn write_fmt(args: fmt::Arguments) {
    if !is_ready() {
        dbg_fmt(args);
        return;
    }

    let mut serial = SERIAL.lock();
    let _ = serial.write_fmt(args);
}

pub fn panic(info: &PanicInfo) {
    if !is_ready() {
        dbg_fmt(format_args!("panic: {info}\n"));
        return;
    }

    let mut serial = SERIAL.lock();
    let _ = writeln!(serial, "panic: {info}");
}

struct SerialPort {
    data: Port<u8>,
    interrupt_enable: Port<u8>,
    fifo_control: Port<u8>,
    line_control: Port<u8>,
    modem_control: Port<u8>,
    line_status: Port<u8>,
}

impl SerialPort {
    const fn new(base: u16) -> Self {
        Self {
            data: Port::new(base),
            interrupt_enable: Port::new(base + 1),
            fifo_control: Port::new(base + 2),
            line_control: Port::new(base + 3),
            modem_control: Port::new(base + 4),
            line_status: Port::new(base + 5),
        }
    }

    unsafe fn init(&mut self) {
        self.interrupt_enable.write(0x00);
        self.line_control.write(0x80);
        self.data.write(0x03);
        self.interrupt_enable.write(0x00);
        self.line_control.write(0x03);
        self.fifo_control.write(0xC7);
        self.modem_control.write(0x0B);
    }

    fn write_byte(&mut self, byte: u8) {
        dbg_hex("serial: write_byte ", byte);
        if byte == b'\n' {
            self.write_byte(b'\r');
        }

        let mut spins: usize = 0;
        loop {
            let status = unsafe { self.line_status.read() };
            if status & 0x20 != 0 || status & 0x40 != 0 {
                break;
            }

            if spins < 8 {
                dbg_hex("serial: wait lsr=", status);
            }
            spins += 1;

            if spins > 1_000_000 {
                dbg_hex("serial: wait timeout lsr=", status);
                break;
            }
        }
        unsafe {
            self.data.write(byte);
        }
        dbg_str("serial: byte sent\n");
    }
}

impl Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}

static SERIAL: Mutex<SerialPort> = Mutex::new(SerialPort::new(COM1_BASE));
static INITIALIZED: AtomicBool = AtomicBool::new(false);

fn is_ready() -> bool {
    INITIALIZED.load(Ordering::Acquire)
}

fn dbg_str(msg: &str) {
    unsafe {
        let mut port = Port::new(0xE9);
        for byte in msg.as_bytes() {
            port.write(*byte);
        }
    }
}

fn dbg_fmt(args: fmt::Arguments) {
    let mut writer = DebugWriter;
    let _ = writer.write_fmt(args);
}

struct DebugWriter;

impl Write for DebugWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        unsafe {
            let mut port = Port::new(0xE9);
            for byte in s.bytes() {
                port.write(byte);
            }
        }
        Ok(())
    }
}

fn dbg_hex(prefix: &str, value: u8) {
    dbg_str(prefix);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let hi = HEX[(value >> 4) as usize];
    let lo = HEX[(value & 0x0F) as usize];
    unsafe {
        let mut port = Port::new(0xE9);
        port.write(hi);
        port.write(lo);
        port.write(b'\n');
    }
}
