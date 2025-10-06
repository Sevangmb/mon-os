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

pub struct WeightsLayout {
    pub total_bytes: usize,
}

impl WeightsLayout {
    pub fn compute(h: &ModelHeader) -> Option<Self> {
        if h.dtype != 0 { // only int8 supported for now
            return None;
        }
        let nl = h.n_layers as usize;
        if nl == 0 { return None; }
        let hidden = h.hidden as usize;
        let last_out = if h.vocab != 0 { h.vocab as usize } else { hidden };
        let mut total: usize = 0;
        for l in 0..nl {
            let in_dim = hidden;
            let out_dim = if l + 1 == nl { last_out } else { hidden };
            // weights int8
            total = total.saturating_add(in_dim.saturating_mul(out_dim));
            // biases i32 per output
            total = total.saturating_add(out_dim.saturating_mul(core::mem::size_of::<i32>()));
        }
        Some(Self { total_bytes: total })
    }
}

pub unsafe fn layer_ptr_int8(base: *const u8, h: &ModelHeader, layer: usize) -> Option<*const i8> {
    if h.dtype != 0 { return None; }
    let nl = h.n_layers as usize;
    if layer >= nl { return None; }
    let hidden = h.hidden as usize;
    let last_out = if h.vocab != 0 { h.vocab as usize } else { hidden };
    let mut offset = 0usize;
    for l in 0..layer {
        let in_dim = hidden;
        let out_dim = if l + 1 == nl { last_out } else { hidden };
        offset = offset.saturating_add(in_dim.saturating_mul(out_dim));
        offset = offset.saturating_add(out_dim.saturating_mul(core::mem::size_of::<i32>()));
    }
    let wptr = base.add(ModelHeader::PAYLOAD_OFFSET + offset);
    Some(wptr as *const i8)
}

pub fn layer_dims(h: &ModelHeader, layer: usize) -> Option<(usize, usize)> {
    let nl = h.n_layers as usize;
    if layer >= nl { return None; }
    let hidden = h.hidden as usize;
    let last_out = if h.vocab != 0 { h.vocab as usize } else { hidden };
    let in_dim = hidden;
    let out_dim = if layer + 1 == nl { last_out } else { hidden };
    Some((in_dim, out_dim))
}

pub unsafe fn bias_ptr_i32(base: *const u8, h: &ModelHeader, layer: usize) -> Option<*const i32> {
    if h.dtype != 0 { return None; }
    let nl = h.n_layers as usize;
    if layer >= nl { return None; }
    let hidden = h.hidden as usize;
    let last_out = if h.vocab != 0 { h.vocab as usize } else { hidden };
    // compute offset to start of this layer
    let mut offset = 0usize;
    for l in 0..layer {
        let in_dim = hidden;
        let out_dim = if l + 1 == nl { last_out } else { hidden };
        offset = offset.saturating_add(in_dim.saturating_mul(out_dim));
        offset = offset.saturating_add(out_dim.saturating_mul(core::mem::size_of::<i32>()));
    }
    // add weights of this layer
    let (in_dim, out_dim) = match layer_dims(h, layer) { Some(d) => d, None => return None };
    offset = offset.saturating_add(in_dim.saturating_mul(out_dim));
    let bptr = base.add(ModelHeader::PAYLOAD_OFFSET + offset);
    Some(bptr as *const i32)
}
