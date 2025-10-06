#![allow(dead_code)]

use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::ai_action::{actf, Action, ActionOutcome, ActionType};
use crate::ai_model::{ModelHeader, WeightsLayout, layer_ptr_int8, layer_dims};
use crate::ai_link::AI_MODEL_LEN;
use crate::{idt, pmm};

static AI_RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" {
    fn ai_propose_action(action: *const Action, outcome: *mut ActionOutcome) -> i32;
}

#[inline(always)]
fn idle_hlt() -> ! {
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

unsafe fn load_model(addr: *const u8) -> Option<NonNull<ModelHeader>> {
    let hdr = ModelHeader::read_unaligned(addr, core::mem::size_of::<ModelHeader>())?;
    if !hdr.valid() {
        return None;
    }
    NonNull::new(addr as *mut ModelHeader)
}

#[allow(unused_variables)]
pub unsafe fn matmul_int8(
    a: *const i8,
    b: *const i8,
    out: *mut i32,
    m: usize,
    n: usize,
    k: usize,
) {
    for i in 0..m {
        let a_row = a.add(i * k);
        for j in 0..n {
            let mut acc: i32 = 0;
            for p in 0..k {
                let ai = *a_row.add(p) as i32;
                let bj = *b.add(p * n + j) as i32;
                acc += ai * bj;
            }
            *out.add(i * n + j) = acc;
        }
    }
}

#[derive(Copy, Clone, Default)]
pub struct Telemetry {
    pub irq_errors: u32,
    pub runq: u32,
    pub irq_rate: u32,   // approx ticks per loop
    pub free_kb: u32,
    pub pf_rate: u32,
}

fn gather_telemetry(prev_ticks: &mut u64, prev_pf: &mut u64) -> Telemetry {
    let ticks = idt::timer_ticks();
    let rate = (ticks.saturating_sub(*prev_ticks)) as u32;
    *prev_ticks = ticks;
    let pf = idt::page_faults();
    let pf_rate = (pf.saturating_sub(*prev_pf)) as u32;
    *prev_pf = pf;
    let free_kb = pmm::free_kib() as u32;
    Telemetry { irq_errors: 0, runq: 0, irq_rate: rate, free_kb, pf_rate }
}

fn infer_and_propose(hdr: &ModelHeader, tel: &Telemetry, scratch: &mut [i32; 1024], model_addr: *const u8) -> Action {
    // Build input vector of length hidden
    let hidden = hdr.hidden as usize;
    let mut inbuf_i8 = [0i8; 256];
    let in_slice = &mut inbuf_i8[..hidden.min(inbuf_i8.len())];
    // Very simple features: runq, irq_rate, free_kb (scaled)
    if !in_slice.is_empty() {
        in_slice[0] = tel.runq.min(127) as i8;
    }
    if in_slice.len() > 1 { in_slice[1] = tel.irq_rate.min(127) as i8; }
    if in_slice.len() > 2 { in_slice[2] = ((tel.free_kb / 1024).min(127)) as i8; } // MB approx
    if in_slice.len() > 3 { in_slice[3] = tel.pf_rate.min(127) as i8; }

    // Check model length for weights availability
    let model_len = unsafe { AI_MODEL_LEN };
    let need = WeightsLayout::compute(hdr).map(|w| w.total_bytes + ModelHeader::PAYLOAD_OFFSET).unwrap_or(0);
    let has_weights = need > ModelHeader::PAYLOAD_OFFSET && model_len >= need;

    // Buffer courant (int8) pour les couches, sans allocation
    let mut xbuf = [0i8; 256];
    let len = in_slice.len();
    xbuf[..len].copy_from_slice(in_slice);
    let mut x_len = len;

    if has_weights && hdr.dtype == 0 {
        let nl = hdr.n_layers as usize;
        for l in 0..nl {
            let (in_dim, out_dim) = match layer_dims(hdr, l) { Some(d) => d, None => break };
            if in_dim > x_len || out_dim > 256 || in_dim == 0 || out_dim == 0 { break; }
            // Prepare i32 output in scratch
            let out_ptr = scratch.as_mut_ptr();
            let w_ptr = unsafe { layer_ptr_int8(model_addr, hdr, l).unwrap_or(core::ptr::null()) };
            if w_ptr.is_null() { break; }
            // Do matmul: out = W (out_dim x in_dim) * x (in_dim)
            unsafe {
                for oi in 0..out_dim {
                    let mut acc: i32 = 0;
                    let w_row = w_ptr.add(oi * in_dim);
                    for p in 0..in_dim {
                        let a = *w_row.add(p) as i32;
                        let b = xbuf[p] as i32;
                        acc += a * b;
                    }
                    *out_ptr.add(oi) = acc;
                }
            }
            // ReLU + requantize by >> 6 (un peu plus de dynamique)
            for oi in 0..out_dim {
                let mut v = scratch[oi];
                if v < 0 { v = 0; }
                v >>= 6; // crude scale
                if v > 127 { v = 127; }
                xbuf[oi] = v as i8;
            }
            x_len = out_dim;
        }
    }

    // Score = premier neurone ou 0
    let mut score = if x_len > 0 { xbuf[0] as i32 } else { 0 };
    // Fallback heuristic influence if no weights (or weak): penalize page faults, reward free memory
    if !has_weights {
        let free_mb = (tel.free_kb / 1024) as i32;
        score = tel.runq as i32 + (tel.irq_rate as i32)/2 - (tel.pf_rate as i32) - free_mb/8;
        if score < -127 { score = -127; }
        if score > 127 { score = 127; }
    }
    // Map score to quantum (100..50_000 µs)
    let mut quantum: i32 = 1000 + score * 20; // ±2540 autour de 1ms
    if quantum < 100 { quantum = 100; }
    if quantum > 50_000 { quantum = 50_000; }

    Action {
        kind: ActionType::SetQuantum as u8,
        flags: actf::REQUIRES_SNAPSHOT,
        _r: [0; 2],
        param1: quantum as u64,
        param2: 0,
        param3: 0,
    }
}

#[no_mangle]
pub extern "C" fn ai_agent_main(model_addr: *const u8) -> ! {
    let Some(model) = unsafe { load_model(model_addr) } else { return idle_hlt(); };
    let hdr = unsafe { core::ptr::read_unaligned(model.as_ptr()) };

    let mut scratch: [i32; 1024] = [0; 1024];
    let mut prev_ticks: u64 = idt::timer_ticks();
    let mut prev_pf: u64 = idt::page_faults();

    while AI_RUNNING.load(Ordering::Acquire) {
        let tel = gather_telemetry(&mut prev_ticks, &mut prev_pf);
        let action = infer_and_propose(&hdr, &tel, &mut scratch, model.as_ptr() as *const u8);

        if (action.flags & actf::NEEDS_MANUAL_CONFIRM) != 0 {
            unsafe { core::arch::asm!("hlt"); }
            continue;
        }

        let mut outcome = ActionOutcome::default();
        let _ = unsafe { ai_propose_action(&action as *const _, &mut outcome as *mut _) };
        unsafe { core::arch::asm!("hlt"); }
    }

    idle_hlt()
}
