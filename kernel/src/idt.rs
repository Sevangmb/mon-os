use spin::Once;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::hlt;
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

use crate::{gdt, keyboard, pic, serial, syscall};
use x86_64::PrivilegeLevel;

static IDT: Once<InterruptDescriptorTable> = Once::new();
static IRQ_COUNT: AtomicU64 = AtomicU64::new(0);
static PAGE_FAULTS: AtomicU64 = AtomicU64::new(0);
static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
#[repr(u8)]
enum InterruptIndex {
    Timer = pic::PIC_1_OFFSET,
    Keyboard,
    Cascade,
    Serial2,
    Serial1,
    Lpt2,
    Floppy,
    Lpt1,
    Rtc = pic::PIC_2_OFFSET,
    Acpi,
    Available1,
    Available2,
    Mouse,
    Coprocessor,
    PrimaryAta,
    SecondaryAta,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }

    fn as_usize(self) -> usize {
        self.as_u8() as usize
    }
}

pub fn init() {
    let idt = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        idt.divide_error.set_handler_fn(handlers::divide_error);
        idt.debug.set_handler_fn(handlers::debug);
        idt.non_maskable_interrupt
            .set_handler_fn(handlers::non_maskable_interrupt);
        idt.breakpoint.set_handler_fn(handlers::breakpoint);
        idt.overflow.set_handler_fn(handlers::overflow);
        idt.bound_range_exceeded
            .set_handler_fn(handlers::bound_range_exceeded);
        idt.invalid_opcode.set_handler_fn(handlers::invalid_opcode);
        idt.device_not_available
            .set_handler_fn(handlers::device_not_available);
        unsafe {
            idt.double_fault
                .set_handler_fn(handlers::double_fault)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt.invalid_tss.set_handler_fn(handlers::invalid_tss);
        idt.segment_not_present
            .set_handler_fn(handlers::segment_not_present);
        idt.stack_segment_fault
            .set_handler_fn(handlers::stack_segment_fault);
        idt.general_protection_fault
            .set_handler_fn(handlers::general_protection_fault);
        idt.page_fault.set_handler_fn(handlers::page_fault);
        idt.x87_floating_point
            .set_handler_fn(handlers::x87_floating_point);
        idt.alignment_check
            .set_handler_fn(handlers::alignment_check);
        idt.machine_check.set_handler_fn(handlers::machine_check);
        idt.simd_floating_point
            .set_handler_fn(handlers::simd_floating_point);
        idt.virtualization.set_handler_fn(handlers::virtualization);
        idt.cp_protection_exception
            .set_handler_fn(handlers::cp_protection_exception);
        idt.hv_injection_exception
            .set_handler_fn(handlers::hv_injection_exception);
        idt.vmm_communication_exception
            .set_handler_fn(handlers::vmm_communication_exception);
        idt.security_exception
            .set_handler_fn(handlers::security_exception);

        idt[InterruptIndex::Timer.as_usize()].set_handler_fn(handlers::timer);
        idt[InterruptIndex::Keyboard.as_usize()].set_handler_fn(handlers::keyboard);
        idt[InterruptIndex::Cascade.as_usize()].set_handler_fn(handlers::cascade);
        idt[InterruptIndex::Serial2.as_usize()].set_handler_fn(handlers::serial2);
        idt[InterruptIndex::Serial1.as_usize()].set_handler_fn(handlers::serial1);
        idt[InterruptIndex::Lpt2.as_usize()].set_handler_fn(handlers::lpt2);
        idt[InterruptIndex::Floppy.as_usize()].set_handler_fn(handlers::floppy);
        idt[InterruptIndex::Lpt1.as_usize()].set_handler_fn(handlers::lpt1);
        idt[InterruptIndex::Rtc.as_usize()].set_handler_fn(handlers::rtc);
        idt[InterruptIndex::Acpi.as_usize()].set_handler_fn(handlers::acpi);
        idt[InterruptIndex::Available1.as_usize()].set_handler_fn(handlers::available1);
        idt[InterruptIndex::Available2.as_usize()].set_handler_fn(handlers::available2);
        idt[InterruptIndex::Mouse.as_usize()].set_handler_fn(handlers::mouse);
        idt[InterruptIndex::Coprocessor.as_usize()].set_handler_fn(handlers::coprocessor);
        idt[InterruptIndex::PrimaryAta.as_usize()].set_handler_fn(handlers::primary_ata);
        idt[InterruptIndex::SecondaryAta.as_usize()].set_handler_fn(handlers::secondary_ata);

        syscall::configure_idt(&mut idt, PrivilegeLevel::Ring3);

        idt
    });

    idt.load();
}

mod handlers {
    use super::*;
    use core::sync::atomic::Ordering;
    use x86_64::instructions::port::Port;

    macro_rules! simple_handler {
        ($fn_name:ident, $label:expr) => {
            pub extern "x86-interrupt" fn $fn_name(stack: InterruptStackFrame) {
                report($label, &stack, None);
            }
        };
    }

    macro_rules! error_handler {
        ($fn_name:ident, $label:expr) => {
            pub extern "x86-interrupt" fn $fn_name(stack: InterruptStackFrame, error_code: u64) {
                report($label, &stack, Some(error_code));
            }
        };
    }

    macro_rules! irq_handler {
        ($fn_name:ident, $index:expr) => {
            pub extern "x86-interrupt" fn $fn_name(_stack: InterruptStackFrame) {
                IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
                pic::notify_end_of_interrupt($index.as_u8());
            }
        };
    }

    simple_handler!(divide_error, "Divide Error");
    simple_handler!(debug, "Debug");
    simple_handler!(non_maskable_interrupt, "Non Maskable Interrupt");
    simple_handler!(breakpoint, "Breakpoint");
    simple_handler!(overflow, "Overflow");
    simple_handler!(bound_range_exceeded, "Bound Range Exceeded");
    simple_handler!(invalid_opcode, "Invalid Opcode");
    simple_handler!(device_not_available, "Device Not Available");
    simple_handler!(x87_floating_point, "x87 Floating Point");
    simple_handler!(simd_floating_point, "SIMD Floating Point");
    simple_handler!(virtualization, "Virtualization");
    simple_handler!(hv_injection_exception, "Hypervisor Injection");

    pub extern "x86-interrupt" fn machine_check(stack: InterruptStackFrame) -> ! {
        report("Machine Check", &stack, None);
    }

    error_handler!(invalid_tss, "Invalid TSS");
    error_handler!(segment_not_present, "Segment Not Present");
    error_handler!(stack_segment_fault, "Stack Segment Fault");
    error_handler!(general_protection_fault, "General Protection Fault");
    error_handler!(alignment_check, "Alignment Check");
    error_handler!(cp_protection_exception, "Control Protection");
    error_handler!(vmm_communication_exception, "VMM Communication");
    error_handler!(security_exception, "Security");

    pub extern "x86-interrupt" fn double_fault(stack: InterruptStackFrame, error_code: u64) -> ! {
        report("Double Fault", &stack, Some(error_code));
    }

    pub extern "x86-interrupt" fn page_fault(
        stack: InterruptStackFrame,
        error_code: PageFaultErrorCode,
    ) {
        PAGE_FAULTS.fetch_add(1, Ordering::Relaxed);
        let addr = Cr2::read();
        serial::write_fmt(format_args!(
            "[EXCEPTION] Page Fault\r\n  address: {addr:?}\r\n  error: {error_code:?}\r\n  bits: {:#06b}\r\n",
            error_code.bits()
        ));
        serial::write_fmt(format_args!("{stack:#?}\r\n"));
        halt_loop();
    }

    pub extern "x86-interrupt" fn timer(_stack: InterruptStackFrame) {
        let ticks = super::TIMER_TICKS.fetch_add(1, Ordering::Relaxed) + 1;
        IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
        if ticks % 1000 == 0 {
            debug_line("[irq] timer\n");
        }
        pic::notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }

    pub extern "x86-interrupt" fn keyboard(_stack: InterruptStackFrame) {
        let mut port = Port::new(0x60);
        let scancode: u8 = unsafe { port.read() };
        let trigger = keyboard::handle_scancode(scancode);
        pic::notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
        if let Some(combo) = trigger {
            keyboard::shutdown_via_keyboard(combo);
        }
    }

    irq_handler!(cascade, InterruptIndex::Cascade);
    irq_handler!(serial2, InterruptIndex::Serial2);
    irq_handler!(serial1, InterruptIndex::Serial1);
    irq_handler!(lpt2, InterruptIndex::Lpt2);
    irq_handler!(floppy, InterruptIndex::Floppy);
    irq_handler!(lpt1, InterruptIndex::Lpt1);
    irq_handler!(rtc, InterruptIndex::Rtc);
    irq_handler!(acpi, InterruptIndex::Acpi);
    irq_handler!(available1, InterruptIndex::Available1);
    irq_handler!(available2, InterruptIndex::Available2);
    irq_handler!(mouse, InterruptIndex::Mouse);
    irq_handler!(coprocessor, InterruptIndex::Coprocessor);
    irq_handler!(primary_ata, InterruptIndex::PrimaryAta);
    irq_handler!(secondary_ata, InterruptIndex::SecondaryAta);

    fn debug_line(message: &str) {
        unsafe {
            let mut port = Port::new(0xE9);
            for byte in message.bytes() {
                port.write(byte);
            }
        }
    }

    fn report(label: &str, stack: &InterruptStackFrame, error: Option<u64>) -> ! {
        serial::write_fmt(format_args!("[EXCEPTION] {label}\r\n"));
        if let Some(code) = error {
            serial::write_fmt(format_args!("  code: 0x{code:016x}\r\n"));
        }
        serial::write_fmt(format_args!("{stack:#?}\r\n"));
        halt_loop();
    }

    fn halt_loop() -> ! {
        loop {
            hlt();
        }
    }
}

pub fn timer_ticks() -> u64 { TIMER_TICKS.load(Ordering::Relaxed) }

pub fn irq_count() -> u64 {
    IRQ_COUNT.load(Ordering::Relaxed)
}

pub fn page_faults() -> u64 {
    PAGE_FAULTS.load(Ordering::Relaxed)
}
