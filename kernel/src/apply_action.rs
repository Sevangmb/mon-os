#![allow(dead_code)]

use spin::Mutex;

use crate::ai_action::{Action, ActionOutcome, ActionType};
use crate::journal;
use crate::idt;
use core::sync::atomic::{AtomicBool, Ordering};

static APPLY_LOCK: Mutex<()> = Mutex::new(());
static mut QUANTUM_US: u32 = 1000;
static mut SEQ: u64 = 0;
static SYSTEM_READY: AtomicBool = AtomicBool::new(false);

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ApplyError {
    NotAllowed = 1,
    InvalidParams = 3,
    ExecuteFailed = 4,
    SelfTestFailed = 5,
}

pub type ApplyResult<T> = core::result::Result<T, ApplyError>;

fn is_allowed(kind: u8) -> bool {
    match kind {
        x if x == ActionType::SetQuantum as u8 => true,
        x if x == ActionType::TrimCache as u8 => true,
        _ => false,
    }
}

fn validate_params(a: &Action) -> bool {
    match a.kind {
        x if x == ActionType::SetQuantum as u8 => {
            let us = a.param1 as u32;
            (100..=50_000).contains(&us)
        }
        x if x == ActionType::TrimCache as u8 => {
            let bytes = a.param1 as u64;
            bytes > 0 && bytes <= 16 * 1024 * 1024
        }
        _ => false,
    }
}

fn read_before_state() -> u32 {
    unsafe { QUANTUM_US }
}

fn write_quantum(us: u32) -> bool {
    unsafe { QUANTUM_US = us; }
    true
}

fn self_test_ok() -> bool {
    // Basic liveness check: timer tick advances and no page fault spike within short window
    let start_ticks = idt::timer_ticks();
    let start_pf = idt::page_faults();
    // busy-wait a little (no sleep available here); allow IRQs to fire
    for _ in 0..50_000 {
        core::hint::spin_loop();
        if idt::timer_ticks().saturating_sub(start_ticks) >= 1 {
            break;
        }
    }
    let dt = idt::timer_ticks().saturating_sub(start_ticks);
    let dp = idt::page_faults().saturating_sub(start_pf);
    dt >= 1 && dp == 0
}

fn trim_cache(bytes: u64) -> bool {
    // Stub: no real cache subsystem yet. Simulate quick success.
    let _ = bytes;
    true
}

pub fn apply_action_atomic(seq: u64, a: &Action) -> ApplyResult<()> {
    // Gate actions until the system is fully initialized
    if !SYSTEM_READY.load(Ordering::Acquire) {
        journal::journal_reject(seq, a);
        return Err(ApplyError::NotAllowed);
    }
    if !is_allowed(a.kind) || !validate_params(a) {
        journal::journal_reject(seq, a);
        return Err(ApplyError::NotAllowed);
    }

    let _g = APPLY_LOCK.lock();
    let before = read_before_state();
    journal::journal_intent(seq, a);

    let ok = match a.kind {
        x if x == ActionType::SetQuantum as u8 => write_quantum(a.param1 as u32),
        x if x == ActionType::TrimCache as u8 => trim_cache(a.param1 as u64),
        _ => false,
    };

    if !ok {
        journal::journal_fail(seq, a, ApplyError::ExecuteFailed as u32);
        return Err(ApplyError::ExecuteFailed);
    }

    if self_test_ok() {
        journal::journal_commit(seq, a);
        Ok(())
    } else {
        let _ = write_quantum(before);
        journal::journal_fail(seq, a, ApplyError::SelfTestFailed as u32);
        Err(ApplyError::SelfTestFailed)
    }
}

#[no_mangle]
pub extern "C" fn ai_propose_action(
    action: *const Action,
    outcome: *mut ActionOutcome,
) -> i32 {
    if action.is_null() || outcome.is_null() {
        return -1;
    }
    let a = unsafe { &*action };
    let seq = unsafe {
        let s = SEQ;
        SEQ = SEQ.wrapping_add(1);
        s
    };

    let res = match apply_action_atomic(seq, a) {
        Ok(()) => 0u8,
        Err(e) => match e {
            ApplyError::NotAllowed => 1u8,
            ApplyError::InvalidParams => 3u8,
            ApplyError::ExecuteFailed => 4u8,
            ApplyError::SelfTestFailed => 5u8,
        },
    };

    unsafe {
        (*outcome).result = res;
    }
    0
}

pub fn set_system_ready() {
    SYSTEM_READY.store(true, Ordering::Release);
}
