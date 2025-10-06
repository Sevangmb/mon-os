#![allow(dead_code)]

use crate::ai_action::Action;

#[inline]
fn e9(b: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0xE9u16, in("al") b);
    }
}

fn w(s: &str) {
    for &b in s.as_bytes() {
        e9(b);
    }
}

fn w_u64(mut v: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    if v == 0 {
        e9(b'0');
        return;
    }
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    for &x in &buf[i..] {
        e9(x);
    }
}

fn sp() {
    e9(b' ');
}

fn nl() {
    e9(b'\n');
}

pub fn journal_intent(seq: u64, a: &Action) {
    w("seq=");
    w_u64(seq);
    sp();
    w("INTENT kind=");
    w_u64(a.kind as u64);
    nl();
}

pub fn journal_commit(seq: u64, a: &Action) {
    w("seq=");
    w_u64(seq);
    sp();
    w("APPLY_OK kind=");
    w_u64(a.kind as u64);
    nl();
}

pub fn journal_fail(seq: u64, _a: &Action, code: u32) {
    w("seq=");
    w_u64(seq);
    sp();
    w("APPLY_FAIL code=");
    w_u64(code as u64);
    nl();
}

pub fn journal_reject(seq: u64, a: &Action) {
    w("seq=");
    w_u64(seq);
    sp();
    w("REJECT kind=");
    w_u64(a.kind as u64);
    nl();
}

