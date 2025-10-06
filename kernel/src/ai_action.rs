#![allow(dead_code)]
#![allow(non_camel_case_types)]

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct Action {
    pub kind: u8,
    pub flags: u8,
    pub _r: [u8; 2],
    pub param1: u64,
    pub param2: u64,
    pub param3: u64,
}

pub mod actf {
    pub const REQUIRES_SNAPSHOT: u8 = 1 << 0;
    pub const HIGH_RISK: u8 = 1 << 1;
    pub const NEEDS_MANUAL_CONFIRM: u8 = 1 << 2;
}

#[repr(u8)]
#[derive(Copy, Clone, Debug)]
pub enum ActionType {
    None = 0,
    SetQuantum = 1,
    SetAffinity = 2,
    MigrateTask = 3,
    TrimCache = 4,
    Reboot = 254,
    Halt = 255,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct ActionOutcome {
    pub result: u8, // 0=ACCEPTED,1=REJECTED,2=ROLLED_BACK,3=ERROR
    _rsv: [u8; 3],
    pub selftest_code: u32,
    pub snapshot_id: u64,
}

