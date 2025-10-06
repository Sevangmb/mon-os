#![allow(dead_code)]

use crate::ai_link::AI_MODEL_ADDR;
use crate::ai_model::ModelHeader;

// Boot/loader can set these to point to an initrd image in RAM (cpio newc).
#[no_mangle]
pub static mut INITRD_BASE: *const u8 = core::ptr::null();
#[no_mangle]
pub static mut INITRD_LEN: usize = 0;

// Minimal cpio newc parser (no alloc). Returns pointer to file data if found.
pub unsafe fn cpio_find(base: *const u8, len: usize, name: &str) -> Option<*const u8> {
    if base.is_null() || len < 110 { return None; }
    let mut off: usize = 0;
    while off + 110 <= len {
        // magic "070701"
        let magic = core::slice::from_raw_parts(base.add(off), 6);
        if magic != b"070701" { return None; }
        // helper to read 8-hex field
        let mut read_hex = |field_off: usize| -> Option<u32> {
            let s = core::slice::from_raw_parts(base.add(off + field_off), 8);
            let mut v: u32 = 0;
            for &c in s {
                let d = match c { b'0'..=b'9' => c - b'0', b'a'..=b'f' => 10 + (c - b'a'), b'A'..=b'F' => 10 + (c - b'A'), _ => return None };
                v = (v << 4) | (d as u32);
            }
            Some(v)
        };
        // fields we need: namesize at 94, filesize at 54
        let namesize = read_hex(94)? as usize;
        let filesize = read_hex(54)? as usize;
        let name_off = off + 110;
        if name_off + namesize > len { return None; }
        let name_bytes = core::slice::from_raw_parts(base.add(name_off), namesize);
        // name includes trailing NUL
        let fname = if namesize > 0 { &name_bytes[..namesize.saturating_sub(1)] } else { &name_bytes[..] };
        // Align to 4 after name
        let mut data_off = name_off + namesize;
        data_off = (data_off + 3) & !3; // 4-byte align
        if fname == b"TRAILER!!!" { return None; }
        if fname == name.as_bytes() || fname == [b'.', b'/', name.as_bytes()].concat().as_slice() {
            if data_off + filesize <= len {
                return Some(base.add(data_off));
            } else {
                return None;
            }
        }
        // skip file data + pad to 4
        let mut next = data_off + filesize;
        next = (next + 3) & !3;
        off = next;
    }
    None
}

// Try to locate /ai.mod in initrd and set AI_MODEL_ADDR if valid.
pub unsafe fn try_set_model_from_initrd() {
    if AI_MODEL_ADDR.is_null() && !INITRD_BASE.is_null() && INITRD_LEN >= ModelHeader::SIZE {
        if let Some(ptr) = cpio_find(INITRD_BASE, INITRD_LEN, "ai.mod") {
            // Validate AIMD header
            if let Some(h) = ModelHeader::read_unaligned(ptr, ModelHeader::SIZE) { if h.valid() {
                // Set global symbol
                let p: *const u8 = ptr;
                core::ptr::write_volatile(&mut (AI_MODEL_ADDR as *const u8 as usize) as *mut usize, p as usize);
                // direct assignment is OK too
                extern "C" { static mut AI_MODEL_ADDR: *const u8; }
                AI_MODEL_ADDR = p;
            }}
        }
    }
}

