use pic8259::ChainedPics;
use spin::Mutex;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

pub fn init() {
    let mut pics = PICS.lock();
    unsafe {
        pics.initialize();
        pics.write_masks(0b1111_1100, 0xFF);
    }
}

pub fn notify_end_of_interrupt(irq: u8) {
    unsafe { PICS.lock().notify_end_of_interrupt(irq) };
}
