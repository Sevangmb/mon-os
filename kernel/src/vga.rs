use core::fmt::{self, Write};
use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};
use spin::Mutex;

const VGA_BUFFER_ADDRESS: usize = 0xb8000;
const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;
const DEFAULT_STYLE: u8 = 0x0f; // white on black

static CONSOLE: Mutex<Console> = Mutex::new(Console::new());

pub fn init() {
    CONSOLE.lock().clear();
}

pub fn write_str(message: &str) {
    let mut console = CONSOLE.lock();
    console.write_str(message);
}

pub fn write_line(message: &str) {
    let mut console = CONSOLE.lock();
    console.write_str(message);
    console.new_line();
}

pub fn put_char(c: char) {
    let mut console = CONSOLE.lock();
    let _ = console.write_char(c);
}

pub fn backspace() {
    let mut console = CONSOLE.lock();
    console.backspace();
}

pub fn set_style(style: u8) {
    CONSOLE.lock().style = style;
}

pub fn fmt(args: fmt::Arguments) {
    let mut console = CONSOLE.lock();
    let _ = console.write_fmt(args);
}

pub fn panic(info: &PanicInfo) {
    let mut console = CONSOLE.lock();
    let saved_style = console.style;
    console.style = 0x4f; // white on red for panic
    let _ = writeln!(console, "panic: {info}");
    console.style = saved_style;
}

struct Console {
    column_position: usize,
    row_position: usize,
    style: u8,
}

impl Console {
    const fn new() -> Self {
        Self {
            column_position: 0,
            row_position: 0,
            style: DEFAULT_STYLE,
        }
    }

    fn clear(&mut self) {
        for row in 0..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                self.write_entry_at(b' ', self.style, row, col);
            }
        }
        self.column_position = 0;
        self.row_position = 0;
    }

    fn write_str(&mut self, message: &str) {
        for byte in message.bytes() {
            match byte {
                b'\n' => self.new_line(),
                b'\r' => self.column_position = 0,
                b => self.write_byte(b),
            }
        }
    }

    fn write_byte(&mut self, byte: u8) {
        if self.column_position >= BUFFER_WIDTH {
            self.new_line();
        }

        self.write_entry_at(byte, self.style, self.row_position, self.column_position);
        self.column_position += 1;
    }

    fn new_line(&mut self) {
        self.column_position = 0;
        if self.row_position < BUFFER_HEIGHT - 1 {
            self.row_position += 1;
            return;
        }

        // Scroll up by one line
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let entry = self.read_entry_at(row, col);
                self.write_entry_at(entry as u8, (entry >> 8) as u8, row - 1, col);
            }
        }

        for col in 0..BUFFER_WIDTH {
            self.write_entry_at(b' ', self.style, BUFFER_HEIGHT - 1, col);
        }
    }

    fn backspace(&mut self) {
        if self.column_position > 0 {
            self.column_position -= 1;
            self.write_entry_at(b' ', self.style, self.row_position, self.column_position);
        } else if self.row_position > 0 {
            // Move to end of previous line
            self.row_position -= 1;
            self.column_position = BUFFER_WIDTH - 1;
            self.write_entry_at(b' ', self.style, self.row_position, self.column_position);
        }
    }

    fn buffer_ptr(row: usize, col: usize) -> *mut u16 {
        (VGA_BUFFER_ADDRESS + (row * BUFFER_WIDTH + col) * 2) as *mut u16
    }

    fn write_entry_at(&self, byte: u8, style: u8, row: usize, col: usize) {
        let value = ((style as u16) << 8) | byte as u16;
        unsafe {
            write_volatile(Self::buffer_ptr(row, col), value);
        }
    }

    fn read_entry_at(&self, row: usize, col: usize) -> u16 {
        unsafe { read_volatile(Self::buffer_ptr(row, col)) }
    }
}

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str(s);
        Ok(())
    }

    fn write_char(&mut self, c: char) -> fmt::Result {
        if c == '\n' {
            self.new_line();
        } else {
            let mut buffer = [0u8; 4];
            let encoded = c.encode_utf8(&mut buffer);
            for byte in encoded.bytes() {
                self.write_byte(byte);
            }
        }
        Ok(())
    }
}
