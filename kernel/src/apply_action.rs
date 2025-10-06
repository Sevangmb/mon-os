#![allow(dead_code)]

use spin::Mutex;

use crate::ai_action::{Action, ActionOutcome, ActionType};
use crate::journal;

static APPLY_LOCK: Mutex<()> = Mutex::new(());
static mut QUANTUM_US: u32 = 1000;
static mut SEQ: u64 = 0;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ApplyError {
    NotAllowed = 1,
    InvalidParams = 3,
    ExecuteFailed = 4,
    SelfTestFailed = 5,
}

pub type ApplyResult<T> = core::result::Result<T, ApplyError>;

fn is_allowed(kind: u8) -> bool {
    matches!(kind, x if x == ActionType::SetQuantum as u8)
}

fn validate_params(a: &Action) -> bool {
    match a.kind {
        x if x == ActionType::SetQuantum as u8 => {
            let us = a.param1 as u32;
            (100..=50_000).contains(&us)
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
    true
}

pub fn apply_action_atomic(seq: u64, a: &Action) -> ApplyResult<()> {
    if !is_allowed(a.kind) || !validate_params(a) {
        journal::journal_reject(seq, a);
        return Err(ApplyError::NotAllowed);
    }

    let _g = APPLY_LOCK.lock();
    let before = read_before_state();
    journal::journal_intent(seq, a);

    let ok = match a.kind {
        x if x == ActionType::SetQuantum as u8 => write_quantum(a.param1 as u32),
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

