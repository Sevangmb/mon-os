use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use x86_64::PrivilegeLevel;

use crate::serial;

pub const SYSCALL_VECTOR: usize = 0x80;

pub fn configure_idt(idt: &mut InterruptDescriptorTable, user_level: PrivilegeLevel) {
    idt[SYSCALL_VECTOR]
        .set_handler_fn(syscall_entry)
        .set_privilege_level(user_level);
}

extern "x86-interrupt" fn syscall_entry(stack: InterruptStackFrame) {
    serial::write_str("[syscall] entry\r\n");
    serial::write_fmt(format_args!(
        "  user_stack: {:#018x}\r\n",
        stack.stack_pointer.as_u64()
    ));
}

pub fn init() {
    serial::write_str("syscall: ready\r\n");
}
