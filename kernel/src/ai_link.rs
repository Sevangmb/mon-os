#![allow(dead_code)]

// Weak symbol that a future loader/boot stage can set to point to the model in RAM.
// Default is null, which disables the agent startup path.
#[no_mangle]
pub static mut AI_MODEL_ADDR: *const u8 = core::ptr::null();

#[no_mangle]
pub static mut AI_MODEL_LEN: usize = 0;

// Initrd (cpio newc) base pointer and length provided by stage2/boot.
// Always available so that RAMFS and shell can read from it.
#[no_mangle]
pub static mut INITRD_BASE: *const u8 = core::ptr::null();
#[no_mangle]
pub static mut INITRD_LEN: usize = 0;
