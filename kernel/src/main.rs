#![feature(abi_x86_interrupt)]
#![no_std]
#![cfg_attr(not(test), no_main)]

#[cfg(all(test, not(target_os = "none")))]
extern crate std;

mod bootinfo;
mod gdt;
mod idt;
mod keyboard;
mod pci;
mod pic;
mod pmm;
mod serial;
mod syscall;
mod vga;
mod xhci;
mod ai_action;
#[cfg(feature = "ai_agent")]
mod ai_agent;
mod ai_model;
mod journal;
mod apply_action;
mod ai_link;
#[cfg(feature = "ai_agent")]
mod ai_initrd;
#[cfg(feature = "ai_agent")]
mod task;
mod ramfs;
mod shell;

use bootinfo::BootInfo;
use core::panic::PanicInfo;
use x86_64::instructions::port::Port;
use x86_64::instructions::{hlt, interrupts};
#[cfg(feature = "ai_agent")]
use core::ptr;

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(boot_info_ptr: *const BootInfo) -> ! {
    let boot_info = unsafe { boot_info_ptr.as_ref().expect("boot info pointer") };
    kernel_main(boot_info)
}

#[no_mangle]
pub extern "C" fn kernel_main(boot_info: &BootInfo) -> ! {
    debug_out("kmain: entry\n");

    gdt::init();
    debug_out("kmain: gdt\n");

    serial::init();
    debug_out("kmain: serial\n");

    // Early IA agent scheduling (before IDT/PIC): best-effort steps
    #[cfg(feature = "ai_agent")]
    {
        use crate::apply_action::set_system_ready; // will be called later
        extern "C" {
            static mut AI_MODEL_ADDR: *const u8;
            static mut INITRD_BASE: *const u8;
            static mut INITRD_LEN: usize;
        }
        unsafe {
            // Propagate initrd from BootInfo and try locating the model early
            INITRD_BASE = boot_info.initrd_base() as *const u8;
            INITRD_LEN = boot_info.initrd_len() as usize;
            if !INITRD_BASE.is_null() && INITRD_LEN > 0 {
                ai_initrd::try_set_model_from_initrd();
            }
            if !AI_MODEL_ADDR.is_null() {
                serial::write_str("[ai] early scheduling agent task\r\n");
                let _ = task::register(|| ai_agent::step());
                // Give it a first step opportunity
                task::run_once();
            } else {
                serial::write_str("[ai] model addr not set; agent inactive\r\n");
            }
        }
    }

    idt::init();
    debug_out("kmain: idt\n");
    #[cfg(feature = "ai_agent")]
    { task::run_once(); }

    syscall::init();
    debug_out("kmain: syscall\n");
    #[cfg(feature = "ai_agent")]
    { task::run_once(); }

    serial::write_str("Hello Kernel\r\n");
    debug_out("kmain: wrote serial\n");

    vga::init();
    vga::set_style(0x1f); // white on blue for headline
    vga::write_line("Hello Kernel");
    vga::set_style(0x0f);
    debug_out("kmain: wrote vga\n");

    pic::init();
    debug_out("kmain: pic\n");
    #[cfg(feature = "ai_agent")]
    { task::run_once(); }

    pmm::init(boot_info);
    log_memory_map(boot_info);
    log_usb_controllers();
    #[cfg(feature = "ai_agent")]
    { task::run_once(); }

    interrupts::enable();
    debug_out("kmain: interrupts on\n");
    #[cfg(feature = "ai_agent")]
    {
        // Now the system is considered stable for transactional actions
        crate::apply_action::set_system_ready();
    }

    // Start shell prompt (simple serial/VGA)
    shell::start();


    #[cfg(feature = "trigger_breakpoint")]
    trigger_breakpoint();

    #[cfg(feature = "qemu_exit")]
    exit_qemu(0);

    #[cfg(not(feature = "qemu_exit"))]
    loop {
        xhci::poll_events();
        #[cfg(feature = "ai_agent")]
        {
            task::run_once();
        }
        shell::step();
        hlt();
    }
}

#[cfg(feature = "trigger_breakpoint")]
fn trigger_breakpoint() {
    interrupts::int3();
}

fn log_memory_map(boot_info: &BootInfo) {
    debug_out("kmain: memmap\n");

    let mut regions = 0u64;
    let mut usable_bytes = 0u64;

    unsafe {
        for region in boot_info.memory_map() {
            regions += 1;
            if region.is_usable() {
                usable_bytes = usable_bytes.saturating_add(region.length);
            }

            let end = region.base_addr.saturating_add(region.length);
            let kind = region.kind();
            serial::write_fmt(format_args!(
                "[mem] {:#016x}-{:#016x} {} (type {:#x}, attr {:#x})\r\n",
                region.base_addr,
                end,
                kind.as_str(),
                region.region_type,
                region.attributes,
            ));
        }
    }

    let usable_kib = usable_bytes / 1024;
    serial::write_fmt(format_args!(
        "[mem] usable: {usable_kib} KiB across {regions} entries\r\n",
        usable_kib = usable_kib,
        regions = regions,
    ));
    debug_out("kmain: memmap done\n");
}

fn log_usb_controllers() {
    debug_out("kmain: pci scan\n");
    let mut found = 0usize;
    pci::find_usb_controllers(|addr| {
        found += 1;
        let vendor = pci::vendor_id(addr);
        let device = pci::device_id(addr);
        let class = pci::class_code(addr);
        let subclass = pci::subclass(addr);
        let prog_if = pci::prog_if(addr);
        serial::write_fmt(format_args!(
            "[pci] usb {} vendor={:04x} device={:04x} class={:02x} sub={:02x} if={:02x}\r\n",
            addr, vendor, device, class, subclass, prog_if
        ));

        if prog_if == 0x30 {
            match pci::bar(addr, 0) {
                Some(bar) if bar.is_memory => unsafe {
                    match xhci::inspect(bar.base) {
                        Some(info) => {
                            serial::write_fmt(format_args!(
                                "[xhci] base={:#016x} caplen={} version={:04x} slots={} ports={} ctx={} dboff={:#x} rtsoff={:#x}\r\n",
                                info.base,
                                info.cap_length,
                                info.hci_version,
                                info.max_slots(),
                                info.max_ports(),
                                info.context_size(),
                                info.dboff,
                                info.rtsoff,
                            ));
                            match xhci::init_controller(info) {
                                Ok(()) => {
                                    serial::write_str("[xhci] controller initialized\r\n");
                                    xhci::report_ports();
                                    let _ = xhci::poll_events();
                                    if !xhci::ensure_first_port_enabled() {
                                        serial::write_str("[xhci] no enabled port\r\n");
                                    }
                                    if let Some(slot) = xhci::enable_slot() {
                                        serial::write_fmt(format_args!(
                                            "[xhci] slot {} enabled\r\n",
                                            slot
                                        ));
                                        if xhci::address_device(slot) {
                                            serial::write_str("[xhci] device addressed\r\n");
                                            if let Some(dev_desc_phys) = xhci::get_device_descriptor(slot) {
                                                serial::write_fmt(format_args!(
                                                    "[xhci] device descriptor at {:#x}\r\n",
                                                    dev_desc_phys
                                                ));
                                                if let Some((hdr_phys, total_len, cfg_val)) = xhci::get_configuration_descriptor_header(slot) {
                                                    serial::write_fmt(format_args!(
                                                        "[xhci] config header at {:#x} total_len={} cfg={}\r\n",
                                                        hdr_phys, total_len, cfg_val
                                                    ));
                                                    if let Some(cfg_phys) = xhci::get_configuration_descriptor(slot, total_len) {
                                                        serial::write_fmt(format_args!(
                                                            "[xhci] config descriptor at {:#x}\r\n",
                                                            cfg_phys
                                                        ));
                                                        if xhci::set_configuration(slot, cfg_val) {
                                                            serial::write_str("[xhci] configuration set\r\n");
                                                            if let Some((ep_addr, maxp, interval)) = xhci::parse_hid_keyboard_endpoint(cfg_phys, total_len) {
                                                                serial::write_fmt(format_args!(
                                                                    "[hid] keyboard ep={:#x} maxp={} interval={}\r\n",
                                                                    ep_addr, maxp, interval
                                                                ));
                                                                if xhci::configure_interrupt_in_endpoint(slot, ep_addr, maxp, interval) {
                                                                    serial::write_str("[hid] interrupt endpoint configured\r\n");
                                                                    if xhci::start_hid_polling(slot, ep_addr, maxp) {
                                                                        serial::write_str("[hid] polling started\r\n");
                                                                    } else {
                                                                        serial::write_str("[hid] failed to start polling\r\n");
                                                                    }
                                                                } else {
                                                                    serial::write_str("[hid] configure endpoint failed\r\n");
                                                                }
                                                            } else {
                                                                serial::write_str("[hid] no keyboard endpoint found\r\n");
                                                            }
                                                        } else {
                                                            serial::write_str("[xhci] set configuration failed\r\n");
                                                        }
                                                    } else {
                                                        serial::write_str("[xhci] failed to read full config descriptor\r\n");
                                                    }
                                                } else {
                                                    serial::write_str("[xhci] failed to read config header\r\n");
                                                }
                                            } else {
                                                serial::write_str("[xhci] failed to read device descriptor\r\n");
                                            }
                                        } else {
                                            serial::write_str("[xhci] address device failed\r\n");
                                        }
                                    } else {
                                        serial::write_str("[xhci] enable slot failed\r\n");
                                    }
                                    xhci::poll_events();
                                }
                                Err(err) => serial::write_fmt(format_args!(
                                    "[xhci] init failed: {}\r\n",
                                    err
                                )),
                            }
                        }
                        None => {
                            serial::write_str("[xhci] failed to read capability registers\r\n");
                        }
                    }
                },
                Some(_) => serial::write_str("[xhci] bar0 is not memory-mapped\r\n"),
                None => serial::write_str("[xhci] missing bar0\r\n"),
            }
        }
    });

    if found == 0 {
        serial::write_str("[pci] no usb controllers found\r\n");
    }
    debug_out("kmain: pci scan done\n");
}

fn debug_out(msg: &str) {
    unsafe {
        let mut port = Port::new(0xE9);
        for byte in msg.bytes() {
            port.write(byte);
        }
    }
}

pub fn exit_qemu(code: u32) -> ! {
    unsafe {
        let mut port = Port::<u32>::new(0xF4);
        port.write((code << 1) | 1);
    }

    loop {
        hlt();
    }
}

#[cfg_attr(any(not(test), target_os = "none"), panic_handler)]
fn panic(info: &PanicInfo) -> ! {
    serial::panic(info);
    vga::panic(info);
    loop {
        hlt();
    }
}
