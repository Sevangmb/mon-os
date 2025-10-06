#![allow(dead_code)]

// Weak symbol that a future loader/boot stage can set to point to the model in RAM.
// Default is null, which disables the agent startup path.
#[no_mangle]
pub static mut AI_MODEL_ADDR: *const u8 = core::ptr::null();

#[no_mangle]
pub static mut AI_MODEL_LEN: usize = 0;
