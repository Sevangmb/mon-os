#![allow(dead_code)]

use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::ai_action::{actf, Action, ActionOutcome, ActionType};
use crate::ai_model::ModelHeader;

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
}

fn gather_telemetry() -> Telemetry {
    Telemetry::default()
}

fn infer_and_propose(_hdr: &ModelHeader, _tel: &Telemetry, _scratch: &mut [i32; 1024]) -> Action {
    let quantum_us = 1000u32;
    Action {
        kind: ActionType::SetQuantum as u8,
        flags: actf::REQUIRES_SNAPSHOT,
        _r: [0; 2],
        param1: quantum_us as u64,
        param2: 0,
        param3: 0,
    }
}

#[no_mangle]
pub extern "C" fn ai_agent_main(model_addr: *const u8) -> ! {
    let Some(model) = unsafe { load_model(model_addr) } else { return idle_hlt(); };
    let hdr = unsafe { core::ptr::read_unaligned(model.as_ptr()) };

    let mut scratch: [i32; 1024] = [0; 1024];

    while AI_RUNNING.load(Ordering::Acquire) {
        let tel = gather_telemetry();
        let action = infer_and_propose(&hdr, &tel, &mut scratch);

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

