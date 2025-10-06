#![allow(dead_code)]

use core::mem::size_of;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct ModelHeader {
    pub magic: [u8; 4], // b"AIMD"
    pub n_layers: u16,
    pub hidden: u16,
    pub vocab: u32,
    pub dtype: u8, // 0=int8, 1=int4
    pub _res: [u8; 3],
}

impl ModelHeader {
    pub const MAGIC: [u8; 4] = *b"AIMD";
    pub const SIZE: usize = 16;
    pub const PAYLOAD_OFFSET: usize = 0x10;

    #[inline]
    pub fn valid(&self) -> bool {
        self.magic == Self::MAGIC
            && self.n_layers >= 1
            && self.hidden >= 1
            && (self.dtype == 0 || self.dtype == 1)
    }

    #[inline]
    pub unsafe fn read_unaligned(ptr: *const u8, len: usize) -> Option<Self> {
        if ptr.is_null() || len < size_of::<Self>() {
            return None;
        }
        Some((ptr as *const Self).read_unaligned())
    }
}

