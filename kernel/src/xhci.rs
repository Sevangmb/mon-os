use crate::pmm;
use crate::vga;
use crate::serial;
use bitflags::bitflags;
use core::hint::spin_loop;
use core::marker::PhantomData;
use core::mem::size_of;
use core::ptr::{read_volatile, write_volatile, NonNull};
use core::slice;
use core::sync::atomic::{compiler_fence, Ordering as FenceOrdering};
use spin::{Mutex, Once};

bitflags! {
    pub struct UsbCmd: u32 {
        const RUN_STOP = 1 << 0;
        const HOST_CONTROLLER_RESET = 1 << 1;
        const INTERRUPTER_ENABLE = 1 << 2;
        const HOST_SYSTEM_ERROR_ENABLE = 1 << 3;
        const LIGHT_HOST_CONTROLLER_RESET = 1 << 7;
    }
}

bitflags! {
    pub struct UsbSts: u32 {
        const HOST_CONTROLLER_HALTED = 1 << 0;
        const HOST_SYSTEM_ERROR = 1 << 2;
        const EVENT_INTERRUPT = 1 << 3;
        const PORT_CHANGE_DETECT = 1 << 4;
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct XhciInfo {
    pub base: u64,
    pub cap_length: u8,
    pub hci_version: u16,
    pub hcsparams1: u32,
    pub hcsparams2: u32,
    pub hcsparams3: u32,
    pub hccparams1: u32,
    pub dboff: u32,
    pub rtsoff: u32,
}

impl XhciInfo {
    pub fn max_slots(&self) -> u8 {
        (self.hcsparams1 & 0xFF) as u8
    }

    pub fn max_ports(&self) -> u8 {
        ((self.hcsparams1 >> 24) & 0xFF) as u8
    }

    pub fn context_size(&self) -> u16 {
        if (self.hccparams1 & (1 << 2)) != 0 {
            64
        } else {
            32
        }
    }
}

#[allow(dead_code)]
struct ControllerState {
    info: XhciInfo,
    command_ring_phys: u64,
    command_ring_len: usize,
    command_ring_enqueue: usize,
    command_ring_cycle: bool,
    event_ring_phys: u64,
    event_ring_len: usize,
    event_ring_dequeue: usize,
    event_ring_cycle: bool,
    dcbaa_phys: u64,
    erst_phys: u64,
    last_completion_code: Option<u8>,
    last_completed_slot: Option<u8>,
    last_transfer_code: Option<u8>,
    last_transfer_len: Option<u32>,
    last_transfer_ep: Option<u8>,
    last_transfer_slot: Option<u8>,
    active_slot: Option<u8>,
    ep0_ring_phys: u64,
    ep0_ring_len: usize,
    ep0_enqueue: usize,
    ep0_cycle: bool,
    intr_ep_addr: u8,
    intr_ep_id: u8,
    intr_ring_phys: u64,
    intr_ring_len: usize,
    intr_enqueue: usize,
    intr_cycle: bool,
    hid_buf_phys: u64,
    hid_buf_len: usize,
}

static CONTROLLER_STATE: Once<Mutex<ControllerState>> = Once::new();

#[allow(dead_code)]
pub struct Xhci {
    cap: XhciInfo,
    base: NonNull<u8>,
}

#[allow(dead_code)]
impl Xhci {
    pub unsafe fn new(info: XhciInfo) -> Option<Self> {
        let base = NonNull::new(info.base as *mut u8)?;
        Some(Self { cap: info, base })
    }

    pub fn info(&self) -> &XhciInfo {
        &self.cap
    }

    pub fn operational(&self) -> OperationalRegs {
        let offset = self.cap.cap_length as usize;
        unsafe { OperationalRegs::new(self.base, offset) }
    }

    pub fn runtime(&self) -> RuntimeRegs {
        let offset = self.cap.rtsoff as usize;
        unsafe { RuntimeRegs::new(self.base, offset) }
    }

    pub fn doorbells(&self) -> DoorbellRegs {
        let offset = self.cap.dboff as usize;
        unsafe { DoorbellRegs::new(self.base, offset) }
    }
}

#[allow(dead_code)]
pub struct OperationalRegs {
    base: NonNull<u8>,
}

#[allow(dead_code)]
impl OperationalRegs {
    pub unsafe fn new(base: NonNull<u8>, offset: usize) -> Self {
        let ptr = NonNull::new_unchecked(base.as_ptr().add(offset));
        Self { base: ptr }
    }

    fn reg_ptr(&self, offset: usize) -> *mut u32 {
        unsafe { self.base.as_ptr().add(offset) as *mut u32 }
    }

    pub fn usbcmd(&self) -> UsbCmd {
        UsbCmd::from_bits_truncate(unsafe { read_volatile(self.reg_ptr(0x00)) })
    }

    pub fn set_usbcmd(&self, value: UsbCmd) {
        unsafe { write_volatile(self.reg_ptr(0x00), value.bits()) };
    }

    pub fn usbsts(&self) -> UsbSts {
        UsbSts::from_bits_truncate(unsafe { read_volatile(self.reg_ptr(0x04)) })
    }

    pub fn clear_usbsts(&self, value: UsbSts) {
        unsafe { write_volatile(self.reg_ptr(0x04), value.bits()) };
    }

    pub fn crcr(&self) -> u64 {
        let low = unsafe { read_volatile(self.reg_ptr(0x18)) } as u64;
        let high = unsafe { read_volatile(self.reg_ptr(0x1C)) } as u64;
        (high << 32) | (low & !0xF)
    }

    pub fn set_crcr(&self, value: u64) {
        unsafe {
            write_volatile(self.reg_ptr(0x18), value as u32);
            write_volatile(self.reg_ptr(0x1C), (value >> 32) as u32);
        }
    }

    pub fn dcbaap(&self) -> u64 {
        let low = unsafe { read_volatile(self.reg_ptr(0x30)) } as u64;
        let high = unsafe { read_volatile(self.reg_ptr(0x34)) } as u64;
        (high << 32) | (low & !0x3F)
    }

    pub fn set_dcbaap(&self, value: u64) {
        unsafe {
            write_volatile(self.reg_ptr(0x30), value as u32);
            write_volatile(self.reg_ptr(0x34), (value >> 32) as u32);
        }
    }

    pub fn config(&self) -> u32 {
        unsafe { read_volatile(self.reg_ptr(0x38)) }
    }

    pub fn set_config(&self, value: u32) {
        unsafe { write_volatile(self.reg_ptr(0x38), value) };
    }

    pub fn port(&self, index: usize) -> PortRegs {
        let offset = 0x400 + index * 0x10;
        unsafe { PortRegs::new(self.base, offset) }
    }
}

#[allow(dead_code)]
pub struct RuntimeRegs {
    base: NonNull<u8>,
}

#[allow(dead_code)]
impl RuntimeRegs {
    pub unsafe fn new(base: NonNull<u8>, offset: usize) -> Self {
        let ptr = NonNull::new_unchecked(base.as_ptr().add(offset));
        Self { base: ptr }
    }

    pub fn interrupter_register_set(&self, index: usize) -> InterrupterRegs {
        let stride = 32; // each IRS is 32 bytes
        let offset = 0x20 + index * stride;
        unsafe { InterrupterRegs::new(NonNull::new_unchecked(self.base.as_ptr().add(offset))) }
    }
}

#[allow(dead_code)]
pub struct InterrupterRegs {
    base: NonNull<u8>,
}

#[allow(dead_code)]
impl InterrupterRegs {
    unsafe fn new(base: NonNull<u8>) -> Self {
        Self { base }
    }

    fn reg_ptr(&self, offset: usize) -> *mut u32 {
        unsafe { self.base.as_ptr().add(offset) as *mut u32 }
    }

    pub fn iman(&self) -> u32 {
        unsafe { read_volatile(self.reg_ptr(0x00)) }
    }

    pub fn set_iman(&self, value: u32) {
        unsafe { write_volatile(self.reg_ptr(0x00), value) };
    }

    pub fn imod(&self) -> u32 {
        unsafe { read_volatile(self.reg_ptr(0x04)) }
    }

    pub fn set_imod(&self, value: u32) {
        unsafe { write_volatile(self.reg_ptr(0x04), value) };
    }

    pub fn erstsz(&self) -> u16 {
        unsafe { read_volatile(self.reg_ptr(0x08)) as u16 }
    }

    pub fn set_erstsz(&self, value: u16) {
        unsafe {
            let current = read_volatile(self.reg_ptr(0x08));
            let new = (current & !0xFFFF) | (value as u32);
            write_volatile(self.reg_ptr(0x08), new);
        }
    }

    pub fn erstba(&self) -> u64 {
        let low = unsafe { read_volatile(self.reg_ptr(0x10)) } as u64;
        let high = unsafe { read_volatile(self.reg_ptr(0x14)) } as u64;
        (high << 32) | (low & !0x3)
    }

    pub fn set_erstba(&self, value: u64) {
        unsafe {
            write_volatile(self.reg_ptr(0x10), value as u32);
            write_volatile(self.reg_ptr(0x14), (value >> 32) as u32);
        }
    }

    pub fn erdp(&self) -> u64 {
        let low = unsafe { read_volatile(self.reg_ptr(0x18)) } as u64;
        let high = unsafe { read_volatile(self.reg_ptr(0x1C)) } as u64;
        (high << 32) | low
    }

    pub fn set_erdp(&self, value: u64) {
        unsafe {
            write_volatile(self.reg_ptr(0x18), value as u32);
            write_volatile(self.reg_ptr(0x1C), (value >> 32) as u32);
        }
    }
}

#[allow(dead_code)]
pub struct DoorbellRegs {
    base: NonNull<u8>,
}

#[allow(dead_code)]
impl DoorbellRegs {
    pub unsafe fn new(base: NonNull<u8>, offset: usize) -> Self {
        let ptr = NonNull::new_unchecked(base.as_ptr().add(offset));
        Self { base: ptr }
    }

    pub fn ring(&self, index: usize, target: u32) {
        compiler_fence(FenceOrdering::SeqCst);
        let ptr = unsafe { self.base.as_ptr().add(index * 4) as *mut u32 };
        unsafe { write_volatile(ptr, target) };
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default)]
#[allow(dead_code)]
pub struct Trb {
    pub parameter: u64,
    pub status: u32,
    pub control: u32,
}

#[allow(dead_code)]
pub struct CommandRing<'a> {
    pub trbs: &'a mut [Trb],
    pub enqueue: usize,
    cycle_state: bool,
}

#[allow(dead_code)]
impl<'a> CommandRing<'a> {
    pub fn new(buffer: &'a mut [Trb]) -> Self {
        Self {
            trbs: buffer,
            enqueue: 0,
            cycle_state: true,
        }
    }

    pub fn cycle_state(&self) -> bool {
        self.cycle_state
    }

    pub fn enqueue_slot(&mut self, trb: Trb) {
        let index = self.enqueue % self.trbs.len();
        self.trbs[index] = trb;
        self.enqueue = (self.enqueue + 1) % self.trbs.len();
        if self.enqueue == 0 {
            self.cycle_state = !self.cycle_state;
        }
    }
}

#[allow(dead_code)]
pub struct EventRingSegment<'a> {
    pub trbs: &'a mut [Trb],
    _marker: PhantomData<&'a mut [Trb]>,
}

#[allow(dead_code)]
impl<'a> EventRingSegment<'a> {
    pub fn new(trbs: &'a mut [Trb]) -> Self {
        Self {
            trbs,
            _marker: PhantomData,
        }
    }
}

pub unsafe fn inspect(base: u64) -> Option<XhciInfo> {
    if base == 0 {
        return None;
    }

    let ptr = NonNull::new(base as *mut u32)?;
    let cap = read(ptr, 0);
    let cap_length = (cap & 0xFF) as u8;
    let hci_version = ((cap >> 16) & 0xFFFF) as u16;
    let hcsparams1 = read(ptr, 1);
    let hcsparams2 = read(ptr, 2);
    let hcsparams3 = read(ptr, 3);
    let hccparams1 = read(ptr, 4);
    let dboff = read(ptr, 5);
    let rtsoff = read(ptr, 6);

    Some(XhciInfo {
        base,
        cap_length,
        hci_version,
        hcsparams1,
        hcsparams2,
        hcsparams3,
        hccparams1,
        dboff,
        rtsoff,
    })
}

unsafe fn read(ptr: NonNull<u32>, index: usize) -> u32 {
    let addr = ptr.as_ptr().add(index);
    read_volatile(addr)
}

const CMD_RING_TRBS: usize = 256;
const EVENT_RING_TRBS: usize = 256;

const TRB_TYPE_LINK: u32 = 6;
const TRB_TYPE_COMMAND_COMPLETION: u8 = 0x21;
const TRB_TYPE_TRANSFER_EVENT: u8 = 0x20;
const TRB_TYPE_PORT_STATUS_CHANGE: u8 = 0x22;
const TRB_TYPE_NO_OP_COMMAND: u32 = 23;
const TRB_TYPE_NORMAL: u32 = 1;
const TRB_TYPE_CONFIGURE_ENDPOINT: u32 = 12;
const TRB_TYPE_ENABLE_SLOT: u32 = 9;
const TRB_TYPE_ADDRESS_DEVICE: u32 = 11;
const TRB_TYPE_SETUP_STAGE: u32 = 2;
const TRB_TYPE_DATA_STAGE: u32 = 3;
const TRB_TYPE_STATUS_STAGE: u32 = 4;

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default)]
struct ErstEntry {
    segment_base: u64,
    segment_size: u32,
    reserved: u32,
}

pub unsafe fn init_controller(info: XhciInfo) -> Result<(), &'static str> {
    let controller = Xhci::new(info).ok_or("xhci: null base")?;
    let op = controller.operational();

    // Stop the controller if it is already running
    let mut cmd = op.usbcmd();
    if cmd.contains(UsbCmd::RUN_STOP) {
        cmd.remove(UsbCmd::RUN_STOP);
        op.set_usbcmd(cmd);
        if !wait_for(|| op.usbsts().contains(UsbSts::HOST_CONTROLLER_HALTED)) {
            return Err("xhci: halt timeout");
        }
    }

    // Reset host controller
    cmd = op.usbcmd();
    cmd.insert(UsbCmd::HOST_CONTROLLER_RESET);
    op.set_usbcmd(cmd);
    if !wait_for(|| !op.usbcmd().contains(UsbCmd::HOST_CONTROLLER_RESET)) {
        return Err("xhci: reset bit stuck");
    }
    if !wait_for(|| op.usbsts().contains(UsbSts::HOST_CONTROLLER_HALTED)) {
        return Err("xhci: reset halt timeout");
    }

    // Allocate command ring
    let cmd_ring_phys = pmm::alloc_aligned((CMD_RING_TRBS * size_of::<Trb>()) as u64, 64)
        .ok_or("xhci: no memory for command ring")?;
    let cmd_ring = unsafe { phys_to_slice_mut::<Trb>(cmd_ring_phys, CMD_RING_TRBS) };
    zero_trbs(cmd_ring);
    init_link_trb(cmd_ring, cmd_ring_phys, true);

    // Allocate DCBAA (slot count + 1 entries)
    let slots = controller.info().max_slots() as usize + 1;
    let dcbaa_size = (slots * size_of::<u64>()) as u64;
    let dcbaa_phys = pmm::alloc_aligned(dcbaa_size, 64).ok_or("xhci: no dcbaa")?;
    zero_phys(dcbaa_phys, dcbaa_size as usize);

    // Allocate event ring and ERST
    let event_ring_phys = pmm::alloc_aligned((EVENT_RING_TRBS * size_of::<Trb>()) as u64, 64)
        .ok_or("xhci: no event ring")?;
    let event_ring = unsafe { phys_to_slice_mut::<Trb>(event_ring_phys, EVENT_RING_TRBS) };
    zero_trbs(event_ring);

    let erst_phys = pmm::alloc_aligned(size_of::<ErstEntry>() as u64, 64).ok_or("xhci: no erst")?;
    let erst = unsafe { phys_to_slice_mut::<ErstEntry>(erst_phys, 1) };
    zero_erst(erst);
    erst[0].segment_base = event_ring_phys;
    erst[0].segment_size = EVENT_RING_TRBS as u32;

    op.set_dcbaap(dcbaa_phys);
    op.set_crcr((cmd_ring_phys & !0xF) | 1); // RCS = 1
    op.set_config((controller.info().max_slots() as u32) & 0xFF);

    // Set up interrupter 0
    let runtime = controller.runtime();
    let ir0 = runtime.interrupter_register_set(0);
    ir0.set_erstsz(1);
    ir0.set_erstba(erst_phys);
    ir0.set_erdp(event_ring_phys);
    ir0.set_iman(ir0.iman() | 1); // enable interrupts
    ir0.set_imod(0);

    // Clear status flags
    op.clear_usbsts(
        UsbSts::EVENT_INTERRUPT | UsbSts::PORT_CHANGE_DETECT | UsbSts::HOST_SYSTEM_ERROR,
    );

    // Start controller
    cmd = op.usbcmd();
    cmd.insert(UsbCmd::RUN_STOP | UsbCmd::INTERRUPTER_ENABLE);
    op.set_usbcmd(cmd);
    if !wait_for(|| !op.usbsts().contains(UsbSts::HOST_CONTROLLER_HALTED)) {
        return Err("xhci: run timeout");
    }

    CONTROLLER_STATE.call_once(|| {
        Mutex::new(ControllerState {
            info,
            command_ring_phys: cmd_ring_phys,
            command_ring_len: CMD_RING_TRBS,
            command_ring_enqueue: 0,
            command_ring_cycle: true,
            event_ring_phys,
            event_ring_len: EVENT_RING_TRBS,
            event_ring_dequeue: 0,
            event_ring_cycle: true,
            dcbaa_phys,
            erst_phys,
            last_completion_code: None,
            last_completed_slot: None,
            last_transfer_code: None,
            last_transfer_len: None,
            last_transfer_ep: None,
            last_transfer_slot: None,
            active_slot: None,
            ep0_ring_phys: 0,
            ep0_ring_len: 0,
            ep0_enqueue: 0,
            ep0_cycle: true,
            intr_ep_addr: 0,
            intr_ep_id: 0,
            intr_ring_phys: 0,
            intr_ring_len: 0,
            intr_enqueue: 0,
            intr_cycle: true,
            hid_buf_phys: 0,
            hid_buf_len: 0,
        })
    });

    serial::write_fmt(format_args!(
        "[xhci] runtime ready cr={:#x} erst={:#x} erdp={:#x}\r\n",
        op.crcr(),
        erst_phys,
        event_ring_phys
    ));

    enqueue_noop_command();
    ring_doorbell(0, 0);

    match wait_for_command_completion(1_000_000) {
        Some((code, slot)) => serial::write_fmt(format_args!(
            "[xhci] command completed code={:#x} slot={}\r\n",
            code, slot
        )),
        None => serial::write_str("[xhci] command timeout\r\n"),
    }

    serial::write_fmt(format_args!("[xhci] usbsts={:#x}\r\n", op.usbsts().bits()));

    Ok(())
}

fn zero_trbs(trbs: &mut [Trb]) {
    for trb in trbs.iter_mut() {
        *trb = Trb::default();
    }
}

fn init_link_trb(trbs: &mut [Trb], base_phys: u64, toggle: bool) {
    if let Some(link) = trbs.last_mut() {
        link.parameter = base_phys;
        let mut control = (TRB_TYPE_LINK & 0x3F) << 10;
        control |= 1; // cycle bit
        if toggle {
            control |= 1 << 1;
        }
        link.control = control;
    }
}

unsafe fn phys_to_slice_mut<T>(phys: u64, entries: usize) -> &'static mut [T] {
    slice::from_raw_parts_mut(phys as *mut T, entries)
}

unsafe fn phys_to_mut_ptr(phys: u64) -> *mut u8 {
    phys as *mut u8
}

fn zero_phys(phys: u64, size: usize) {
    unsafe {
        core::ptr::write_bytes(phys_to_mut_ptr(phys), 0, size);
    }
}

fn zero_erst(erst: &mut [ErstEntry]) {
    for entry in erst.iter_mut() {
        *entry = ErstEntry::default();
    }
}

pub fn report_ports() {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let info = {
            let state = state_lock.lock();
            state.info
        };
        unsafe {
            if let Some(controller) = Xhci::new(info) {
                let op = controller.operational();
                for port in 0..info.max_ports() {
                    let regs = op.port(port as usize);
                    let sc = regs.portsc();
                    let ccs = (sc & 0x1) != 0;
                    let ped = (sc & 0x2) != 0;
                    let speed = (sc >> 10) & 0xF;
                    let pls = (sc >> 5) & 0xF;
                    serial::write_fmt(format_args!(
                        "[xhci] port{} sc={:#010x} ccs={} ped={} speed={} pls={}\r\n",
                        port + 1,
                        sc,
                        ccs as u8,
                        ped as u8,
                        speed,
                        pls
                    ));
                }
            }
        }
    }
}

pub fn poll_events() -> bool {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let mut state = state_lock.lock();
        unsafe {
            if let Some(controller) = Xhci::new(state.info) {
                let runtime = controller.runtime();
                let ir0 = runtime.interrupter_register_set(0);
                let ring = phys_to_slice_mut::<Trb>(state.event_ring_phys, state.event_ring_len);
                let trb_size = size_of::<Trb>() as u64;
                let mut processed = false;

                loop {
                    let index = state.event_ring_dequeue;
                    let trb = read_volatile(&ring[index]);
                    let cycle = (trb.control & 1) != 0;
                    if cycle != state.event_ring_cycle {
                        break;
                    }

                    let trb_type = ((trb.control >> 10) & 0x3F) as u8;
                    handle_event(&mut state, trb_type, &trb);
                    processed = true;

                    state.event_ring_dequeue =
                        (state.event_ring_dequeue + 1) % state.event_ring_len;
                    if state.event_ring_dequeue == 0 {
                        state.event_ring_cycle = !state.event_ring_cycle;
                    }

                    let new_erdp =
                        state.event_ring_phys + state.event_ring_dequeue as u64 * trb_size;
                    ir0.set_erdp(new_erdp | (1 << 3));
                }
                return processed;
            }
        }
    }
    false
}

pub fn wait_for_command_completion(iterations: usize) -> Option<(u8, u8)> {
    if CONTROLLER_STATE.get().is_none() {
        return None;
    }

    for _ in 0..iterations {
        let _ = poll_events();
        if let Some(state_lock) = CONTROLLER_STATE.get() {
            let mut state = state_lock.lock();
            if let Some(code) = state.last_completion_code.take() {
                let slot = state.last_completed_slot.take().unwrap_or(0);
                return Some((code, slot));
            }
        }
        spin_loop();
    }
    None
}

fn enqueue_noop_command() {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let mut state = state_lock.lock();
        let usable = state.command_ring_len.saturating_sub(1);
        if usable == 0 {
            serial::write_str("[xhci] command ring unusable\r\n");
            return;
        }

        let index = state.command_ring_enqueue % usable;
        let trbs =
            unsafe { phys_to_slice_mut::<Trb>(state.command_ring_phys, state.command_ring_len) };
        let cycle_bit = if state.command_ring_cycle { 1 } else { 0 };
        trbs[index] = Trb {
            parameter: 0,
            status: 0,
            control: ((TRB_TYPE_NO_OP_COMMAND & 0x3F) << 10) | (1 << 5) | cycle_bit,
        };
        serial::write_fmt(format_args!(
            "[xhci] queued noop index={} cycle={}\r\n",
            index, cycle_bit
        ));
        compiler_fence(FenceOrdering::SeqCst);

        state.command_ring_enqueue = (state.command_ring_enqueue + 1) % usable;
        if state.command_ring_enqueue == 0 {
            state.command_ring_cycle = !state.command_ring_cycle;
        }
    }
}

fn enqueue_command_trb(trb_type: u32, parameter: u64, status: u32) {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let mut state = state_lock.lock();
        let usable = state.command_ring_len.saturating_sub(1);
        if usable == 0 {
            serial::write_str("[xhci] command ring unusable\r\n");
            return;
        }

        let index = state.command_ring_enqueue % usable;
        let trbs = unsafe {
            phys_to_slice_mut::<Trb>(state.command_ring_phys, state.command_ring_len)
        };
        let cycle_bit = if state.command_ring_cycle { 1 } else { 0 };
        trbs[index] = Trb {
            parameter,
            status,
            control: ((trb_type & 0x3F) << 10) | (1 << 5) | cycle_bit,
        };
        compiler_fence(FenceOrdering::SeqCst);

        state.command_ring_enqueue = (state.command_ring_enqueue + 1) % usable;
        if state.command_ring_enqueue == 0 {
            state.command_ring_cycle = !state.command_ring_cycle;
        }
    }
}

fn enqueue_command_trb_slot(trb_type: u32, parameter: u64, status: u32, slot_id: u8) {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let mut state = state_lock.lock();
        let usable = state.command_ring_len.saturating_sub(1);
        if usable == 0 {
            serial::write_str("[xhci] command ring unusable\r\n");
            return;
        }

        let index = state.command_ring_enqueue % usable;
        let trbs = unsafe {
            phys_to_slice_mut::<Trb>(state.command_ring_phys, state.command_ring_len)
        };
        let cycle_bit = if state.command_ring_cycle { 1 } else { 0 };
        let mut control = ((trb_type & 0x3F) << 10) | (1 << 5) | cycle_bit;
        control |= (slot_id as u32) << 16;
        trbs[index] = Trb {
            parameter,
            status,
            control,
        };
        compiler_fence(FenceOrdering::SeqCst);

        state.command_ring_enqueue = (state.command_ring_enqueue + 1) % usable;
        if state.command_ring_enqueue == 0 {
            state.command_ring_cycle = !state.command_ring_cycle;
        }
    }
}

pub fn enable_slot() -> Option<u8> {
    // Queue Enable Slot Command and ring DB0
    enqueue_command_trb(TRB_TYPE_ENABLE_SLOT, 0, 0);
    ring_doorbell(0, 0);
    if let Some((code, slot)) = wait_for_command_completion(1_000_000) {
        serial::write_fmt(format_args!(
            "[xhci] enable slot completion code={:#x} slot={}\r\n",
            code, slot
        ));
        if code == 1 /* Success */ {
            return Some(slot);
        }
    }
    None
}

pub fn address_device(slot_id: u8) -> bool {
    // Allocate and hook Device Context in DCBAA
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let state_info;
        let context_size;
        let dcbaa_phys;
        {
            let state = state_lock.lock();
            state_info = state.info;
            context_size = state.info.context_size() as usize;
            dcbaa_phys = state.dcbaa_phys;
        }

        let dc_entries = 1 /* slot */ + 31; // endpoints
        let dc_bytes = context_size * dc_entries;
        let dc_phys = match pmm::alloc_aligned(dc_bytes as u64, 64) {
            Some(p) => p,
            None => {
                serial::write_str("[xhci] no memory for device context\r\n");
                return false;
            }
        };
        zero_phys(dc_phys, dc_bytes);

        // Install into DCBAA
        let dcbaa_entries = state_info.max_slots() as usize + 1;
        unsafe {
            let dcbaa = phys_to_slice_mut::<u64>(dcbaa_phys, dcbaa_entries);
            dcbaa[slot_id as usize] = dc_phys;
        }

        // Allocate EP0 transfer ring and set it into EP0 context later
        let ep0_trbs = 64usize;
        let ep0_ring_phys = match pmm::alloc_aligned((ep0_trbs * size_of::<Trb>()) as u64, 64) {
            Some(p) => p,
            None => {
                serial::write_str("[xhci] no memory for ep0 ring\r\n");
                return false;
            }
        };
        unsafe {
            let ep0_ring = phys_to_slice_mut::<Trb>(ep0_ring_phys, ep0_trbs);
            zero_trbs(ep0_ring);
            init_link_trb(ep0_ring, ep0_ring_phys, true);
        }

        // Allocate Input Context (ICC + Slot + EP0)
        let ic_entries = 1 /* ICC */ + 1 /* slot */ + 1 /* ep0 */;
        let ic_bytes = context_size * ic_entries;
        let ic_phys = match pmm::alloc_aligned(ic_bytes as u64, 64) {
            Some(p) => p,
            None => {
                serial::write_str("[xhci] no memory for input context\r\n");
                return false;
            }
        };
        zero_phys(ic_phys, ic_bytes);

        // Set Add Context Flags: slot + ep0
        unsafe {
            let ic_ptr = phys_to_mut_ptr(ic_phys) as *mut u32;
            // dword1 at offset 4: Add Context Flags
            write_volatile(ic_ptr.add(1), 0b11);
        }

        // Fill minimal Slot Context and EP0 Context fields
        unsafe {
            let dwords_per_ctx = context_size / 4;
            let slot_ctx = (phys_to_mut_ptr(ic_phys) as *mut u32).add(dwords_per_ctx);
            let ep0_ctx = slot_ctx.add(dwords_per_ctx);

            // Slot Context: set Context Entries = 1 (EP0), RouteString=0
            // DW0 = Route String -> 0
            write_volatile(slot_ctx.add(0), 0);
            // DW1: set speed if available from first connected port
            let speed_code = {
                let mut sp = 0u32;
                if let Some(idx) = find_first_connected_port() {
                    if let Some(state_lock) = CONTROLLER_STATE.get() {
                        let info = { state_lock.lock().info };
                        if let Some(controller) = Xhci::new(info) {
                            let op = controller.operational();
                            let sc = op.port(idx).portsc();
                            sp = ((sc >> 10) & 0xF) as u32;
                        }
                    }
                }
                sp
            };
            // Put speed in a reasonable location (implementation-defined here)
            write_volatile(slot_ctx.add(1), speed_code << 20);
            // DW2: Context Entries = 1 in bits 31:27 (approximate)
            write_volatile(slot_ctx.add(2), 1 << 27);

            // EP0 Context
            // DW0/DW1: set EP Type=Control, MPS per speed
            let mps = match speed_code {
                4 /* SS */ => 512u32,
                3 /* HS */ => 64u32,
                1 /* FS */ | 2 /* LS */ => 8u32,
                _ => 64u32,
            };
            // Store MPS in DW1 upper half (approximate placement)
            write_volatile(ep0_ctx.add(1), mps << 16);
            // Set Dequeue Pointer (DW2/DW3)
            let deq_low = (ep0_ring_phys as u32) & !0xF;
            let deq_high = ((ep0_ring_phys >> 32) as u32);
            write_volatile(ep0_ctx.add(2), deq_low);
            write_volatile(ep0_ctx.add(3), deq_high);
        }

        // Queue Address Device command
        enqueue_command_trb_slot(TRB_TYPE_ADDRESS_DEVICE, ic_phys, 0, slot_id);
        ring_doorbell(0, 0);
        if let Some((code, slot)) = wait_for_command_completion(1_000_000) {
            serial::write_fmt(format_args!(
                "[xhci] address device completion code={:#x} slot={}\r\n",
                code, slot
            ));
            if code == 1 /* Success */ && slot == slot_id {
                if let Some(state_lock) = CONTROLLER_STATE.get() {
                    let mut state = state_lock.lock();
                    state.active_slot = Some(slot_id);
                    state.ep0_ring_phys = ep0_ring_phys;
                    state.ep0_ring_len = ep0_trbs;
                    state.ep0_enqueue = 0;
                    state.ep0_cycle = true;
                }
                return true;
            }
        }
    }
    false
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
struct UsbSetupPacket {
    bmRequestType: u8,
    bRequest: u8,
    wValue: u16,
    wIndex: u16,
    wLength: u16,
}

fn ep0_enqueue_trb(trb: Trb) {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let mut state = state_lock.lock();
        if state.ep0_ring_len == 0 {
            serial::write_str("[xhci] ep0 ring not ready\r\n");
            return;
        }
        let usable = state.ep0_ring_len.saturating_sub(1);
        if usable == 0 { return; }
        let index = state.ep0_enqueue % usable;
        let ring = unsafe { phys_to_slice_mut::<Trb>(state.ep0_ring_phys, state.ep0_ring_len) };
        ring[index] = trb;
        state.ep0_enqueue = (state.ep0_enqueue + 1) % usable;
        if state.ep0_enqueue == 0 {
            state.ep0_cycle = !state.ep0_cycle;
        }
    }
}

fn ep0_cycle_bit() -> u32 {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let state = state_lock.lock();
        return if state.ep0_cycle { 1 } else { 0 };
    }
    1
}

fn ring_ep0(slot_id: u8) {
    ring_doorbell(slot_id, 1);
}

pub fn control_in(slot_id: u8, request_type: u8, request: u8, value: u16, index: u16, length: u16, data_phys: u64) -> bool {
    // Setup stage (IDT, length=8)
    let setup = UsbSetupPacket { bmRequestType: request_type, bRequest: request, wValue: value, wIndex: index, wLength: length };
    let setup_param: u64 = unsafe { core::mem::transmute::<UsbSetupPacket, u64>(setup) };
    let setup_trb = Trb { parameter: setup_param, status: 8, control: ((TRB_TYPE_SETUP_STAGE & 0x3F) << 10) | (1 << 5) | ep0_cycle_bit() };
    ep0_enqueue_trb(setup_trb);

    // Data stage (IN)
    let data_trb = Trb { parameter: data_phys, status: length as u32, control: ((TRB_TYPE_DATA_STAGE & 0x3F) << 10) | (1 << 16) | (1 << 5) | ep0_cycle_bit() };
    ep0_enqueue_trb(data_trb);

    // Status stage (OUT)
    let status_trb = Trb { parameter: 0, status: 0, control: ((TRB_TYPE_STATUS_STAGE & 0x3F) << 10) | (0 << 16) | (1 << 5) | ep0_cycle_bit() };
    ep0_enqueue_trb(status_trb);

    ring_ep0(slot_id);

    // Wait for a transfer event
    for _ in 0..1_000_000 {
        let _ = poll_events();
        if let Some(lock) = CONTROLLER_STATE.get() {
            let mut state = lock.lock();
            if let Some(code) = state.last_transfer_code.take() {
                let len = state.last_transfer_len.take().unwrap_or(0);
                serial::write_fmt(format_args!("[xhci] control_in done code={:#x} len={}\r\n", code, len));
                return code == 1; // Success
            }
        }
        spin_loop();
    }
    false
}

pub fn get_device_descriptor(slot_id: u8) -> Option<u64> {
    let buf_phys = match pmm::alloc_aligned(256, 64) { Some(p) => p, None => { serial::write_str("[xhci] no mem for dev desc\r\n"); return None; } };
    zero_phys(buf_phys, 256);
    let ok = control_in(slot_id, 0x80, 6, (1u16 << 8) | 0, 0, 18, buf_phys);
    if ok { Some(buf_phys) } else { None }
}

pub fn control_no_data(slot_id: u8, request_type: u8, request: u8, value: u16, index: u16) -> bool {
    // Setup only, then Status with IN direction
    let setup = UsbSetupPacket { bmRequestType: request_type, bRequest: request, wValue: value, wIndex: index, wLength: 0 };
    let setup_param: u64 = unsafe { core::mem::transmute::<UsbSetupPacket, u64>(setup) };
    let setup_trb = Trb { parameter: setup_param, status: 8, control: ((TRB_TYPE_SETUP_STAGE & 0x3F) << 10) | (1 << 5) | ep0_cycle_bit() };
    ep0_enqueue_trb(setup_trb);

    // Status stage (IN)
    let status_trb = Trb { parameter: 0, status: 0, control: ((TRB_TYPE_STATUS_STAGE & 0x3F) << 10) | (1 << 16) | (1 << 5) | ep0_cycle_bit() };
    ep0_enqueue_trb(status_trb);

    ring_ep0(slot_id);

    for _ in 0..1_000_000 {
        let _ = poll_events();
        if let Some(lock) = CONTROLLER_STATE.get() {
            let mut state = lock.lock();
            if let Some(code) = state.last_transfer_code.take() {
                serial::write_fmt(format_args!("[xhci] control_out(no-data) done code={:#x}\r\n", code));
                return code == 1;
            }
        }
        spin_loop();
    }
    false
}

pub fn get_configuration_descriptor_header(slot_id: u8) -> Option<(u64, u16, u8)> {
    // Read first 9 bytes to get wTotalLength and bConfigurationValue
    let buf_phys = match pmm::alloc_aligned(64, 64) { Some(p) => p, None => { serial::write_str("[xhci] no mem for cfg head\r\n"); return None; } };
    zero_phys(buf_phys, 64);
    let ok = control_in(slot_id, 0x80, 6, (2u16 << 8) | 0, 0, 9, buf_phys);
    if !ok { return None; }
    unsafe {
        let hdr = phys_to_slice_mut::<u8>(buf_phys, 9);
        let total = (hdr[2] as u16) | ((hdr[3] as u16) << 8);
        let cfg_val = hdr[5];
        Some((buf_phys, total, cfg_val))
    }
}

pub fn get_configuration_descriptor(slot_id: u8, total_len: u16) -> Option<u64> {
    let len = total_len as usize;
    let buf_phys = match pmm::alloc_aligned(len as u64, 64) { Some(p) => p, None => { serial::write_str("[xhci] no mem for cfg desc\r\n"); return None; } };
    zero_phys(buf_phys, len);
    let ok = control_in(slot_id, 0x80, 6, (2u16 << 8) | 0, 0, total_len, buf_phys);
    if ok { Some(buf_phys) } else { None }
}

pub fn parse_hid_keyboard_endpoint(cfg_phys: u64, total_len: u16) -> Option<(u8, u16, u8)> {
    unsafe {
        let bytes = phys_to_slice_mut::<u8>(cfg_phys, total_len as usize);
        let mut i = 0usize;
        let mut in_hid_iface = false;
        while i + 2 <= bytes.len() {
            let b_len = bytes[i] as usize;
            if b_len == 0 { break; }
            if i + b_len > bytes.len() { break; }
            let dtype = bytes[i + 1];
            match dtype {
                4 => {
                    // Interface
                    if b_len >= 9 {
                        let class = bytes[i + 5];
                        let subclass = bytes[i + 6];
                        let proto = bytes[i + 7];
                        in_hid_iface = class == 3 && subclass == 1 && proto == 1; // HID boot keyboard
                    }
                }
                5 => {
                    // Endpoint
                    if in_hid_iface && b_len >= 7 {
                        let addr = bytes[i + 2];
                        let attrs = bytes[i + 3] & 0x3;
                        let maxp = (bytes[i + 4] as u16) | ((bytes[i + 5] as u16) << 8);
                        let interval = bytes[i + 6];
                        if (addr & 0x80) != 0 && attrs == 3 {
                            return Some((addr, maxp, interval));
                        }
                    }
                }
                _ => {}
            }
            i += b_len;
        }
    }
    None
}

pub fn set_configuration(slot_id: u8, cfg_value: u8) -> bool {
    control_no_data(slot_id, 0x00, 9, cfg_value as u16, 0)
}

fn endpoint_id_from_addr(addr: u8) -> u8 {
    let ep = (addr & 0x0F) as u8;
    let dir_in = (addr & 0x80) != 0;
    (ep * 2) + if dir_in { 1 } else { 0 }
}

pub fn configure_interrupt_in_endpoint(slot_id: u8, ep_addr: u8, maxp: u16, _interval: u8) -> bool {
    let ep_id = endpoint_id_from_addr(ep_addr);
    let ctx_size = if let Some(lock) = CONTROLLER_STATE.get() { lock.lock().info.context_size() as usize } else { return false };

    // Allocate interrupt ring
    let ring_trbs = 128usize;
    let ring_phys = match pmm::alloc_aligned((ring_trbs * size_of::<Trb>()) as u64, 64) {
        Some(p) => p,
        None => { serial::write_str("[xhci] no memory for intr ring\r\n"); return false; }
    };
    unsafe {
        let ring = phys_to_slice_mut::<Trb>(ring_phys, ring_trbs);
        zero_trbs(ring);
        init_link_trb(ring, ring_phys, true);
    }

    // Allocate Input Context for Configure Endpoint: ICC + Slot + endpoints up to ep_id
    let ic_entries = 1 + 1 + (ep_id as usize); // rough sizing
    let ic_bytes = ctx_size * ic_entries;
    let ic_phys = match pmm::alloc_aligned(ic_bytes as u64, 64) { Some(p) => p, None => { serial::write_str("[xhci] no memory for conf ic\r\n"); return false; } };
    zero_phys(ic_phys, ic_bytes);

    unsafe {
        let base = phys_to_mut_ptr(ic_phys) as *mut u32;
        // Add Context Flags: set bit for target endpoint id
        // Also keep slot context flagged
        let add_flags = (1u32 << 0) | (1u32 << ep_id);
        write_volatile(base.add(1), add_flags);

        let dwords = ctx_size / 4;
        let slot_ctx = base.add(dwords);
        // Context Entries >= ep_id
        write_volatile(slot_ctx.add(2), (ep_id as u32) << 27);

        // Endpoint context index in array: for EP1 IN -> index 3
        let ep_ctx = slot_ctx.add(dwords * (ep_id as usize));

        // Fill minimal EP context: type=interrupt IN, MaxPacket, Dequeue Ptr
        // DW1: Max Packet Size in bits 31:16 (approx), Interval bits etc ignored here
        let mps = maxp as u32;
        write_volatile(ep_ctx.add(1), mps << 16);
        // DW2/DW3: TR Dequeue Pointer
        let deq_low = (ring_phys as u32) & !0xF;
        let deq_high = (ring_phys >> 32) as u32;
        write_volatile(ep_ctx.add(2), deq_low);
        write_volatile(ep_ctx.add(3), deq_high);
    }

    enqueue_command_trb_slot(TRB_TYPE_CONFIGURE_ENDPOINT, ic_phys, 0, slot_id);
    ring_doorbell(0, 0);
    if let Some((code, slot)) = wait_for_command_completion(1_000_000) {
        serial::write_fmt(format_args!("[xhci] configure ep completion code={:#x} slot={}\r\n", code, slot));
        if code == 1 && slot == slot_id {
            if let Some(lock) = CONTROLLER_STATE.get() {
                let mut st = lock.lock();
                st.intr_ep_addr = ep_addr;
                st.intr_ep_id = ep_id;
                st.intr_ring_phys = ring_phys;
                st.intr_ring_len = ring_trbs;
                st.intr_enqueue = 0;
                st.intr_cycle = true;
            }
            return true;
        }
    }
    false
}

fn intr_cycle_bit() -> u32 {
    if let Some(lock) = CONTROLLER_STATE.get() { let st = lock.lock(); if st.intr_cycle { 1 } else { 0 } } else { 1 }
}

fn intr_enqueue_trb(trb: Trb) {
    if let Some(lock) = CONTROLLER_STATE.get() {
        let mut st = lock.lock();
        let usable = st.intr_ring_len.saturating_sub(1);
        if usable == 0 { return; }
        let idx = st.intr_enqueue % usable;
        let ring = unsafe { phys_to_slice_mut::<Trb>(st.intr_ring_phys, st.intr_ring_len) };
        ring[idx] = trb;
        st.intr_enqueue = (st.intr_enqueue + 1) % usable;
        if st.intr_enqueue == 0 { st.intr_cycle = !st.intr_cycle; }
    }
}

pub fn request_hid_report_once(slot_id: u8, ep_addr: u8, maxp: u16) -> Option<u64> {
    let ep_id = endpoint_id_from_addr(ep_addr);
    let buf_len = maxp as usize;
    let buf_phys = match pmm::alloc_aligned(buf_len as u64, 64) { Some(p) => p, None => { serial::write_str("[xhci] no mem for hid buf\r\n"); return None; } };
    zero_phys(buf_phys, buf_len);
    let trb = Trb { parameter: buf_phys, status: maxp as u32, control: ((TRB_TYPE_NORMAL & 0x3F) << 10) | (1 << 5) | intr_cycle_bit() };
    intr_enqueue_trb(trb);
    ring_doorbell(slot_id, ep_id as u32);

    for _ in 0..1_000_000 {
        let _ = poll_events();
        if let Some(lock) = CONTROLLER_STATE.get() {
            let mut st = lock.lock();
            if let Some(code) = st.last_transfer_code.take() {
                let len = st.last_transfer_len.take().unwrap_or(0) as usize;
                serial::write_fmt(format_args!("[hid] report event code={:#x} len={}\r\n", code, len));
                if code == 1 { return Some(buf_phys); }
                break;
            }
        }
        spin_loop();
    }
    None
}

pub fn start_hid_polling(slot_id: u8, ep_addr: u8, maxp: u16) -> bool {
    if let Some(lock) = CONTROLLER_STATE.get() {
        let mut st = lock.lock();
        if st.intr_ring_len == 0 { return false; }
        if st.hid_buf_phys == 0 {
            let buf_phys = match pmm::alloc_aligned(maxp as u64, 64) { Some(p) => p, None => return false };
            zero_phys(buf_phys, maxp as usize);
            st.hid_buf_phys = buf_phys;
            st.hid_buf_len = maxp as usize;
        }
        let cycle = if st.intr_cycle { 1 } else { 0 };
        let trb = Trb { parameter: st.hid_buf_phys, status: maxp as u32, control: ((TRB_TYPE_NORMAL & 0x3F) << 10) | (1 << 5) | cycle };
        let usable = st.intr_ring_len.saturating_sub(1);
        if usable == 0 { return false; }
        let idx = st.intr_enqueue % usable;
        unsafe { let ring = phys_to_slice_mut::<Trb>(st.intr_ring_phys, st.intr_ring_len); ring[idx] = trb; }
        st.intr_enqueue = (st.intr_enqueue + 1) % usable;
        if st.intr_enqueue == 0 { st.intr_cycle = !st.intr_cycle; }
        ring_doorbell(slot_id, endpoint_id_from_addr(ep_addr) as u32);
        return true;
    }
    false
}

pub fn decode_hid_report(buf_phys: u64, len: usize) {
    unsafe {
        let data = phys_to_slice_mut::<u8>(buf_phys, len);
        serial::write_fmt(format_args!("[hid] data: "));
        for i in 0..len { serial::write_fmt(format_args!("{:02x} ", data[i])); }
        serial::write_str("\r\n");

        // Very small decoder: first key only, ASCII for letters and digits
        let modifiers = data[0];
        let shift = (modifiers & 0x22) != 0; // LShift or RShift
        let key = data[2];
        if let Some(ch) = hid_usage_to_ascii(key, shift) {
            vga::put_char(ch);
        }
    }
}

fn hid_usage_to_ascii(usage: u8, shift: bool) -> Option<char> {
    match usage {
        0x04..=0x1d => {
            let base = if shift { b'A' } else { b'a' };
            let ch = base + (usage - 0x04);
            Some(ch as char)
        }
        0x1e..=0x26 => {
            // 1..9
            let digits = b"123456789";
            let ch = digits[(usage - 0x1e) as usize];
            Some(ch as char)
        }
        0x27 => Some('0'),
        0x28 => { vga::put_char('\n'); None }
        0x2a => { vga::backspace(); None }
        0x2c => Some(' '),
        0x2d => Some(if shift { '_' } else { '-' }),
        0x2e => Some(if shift { '+' } else { '=' }),
        0x2f => Some(if shift { '{' } else { '[' }),
        0x30 => Some(if shift { '}' } else { ']' }),
        0x31 => Some(if shift { '|' } else { '\\' }),
        0x33 => Some(if shift { ':' } else { ';' }),
        0x34 => Some(if shift { '"' } else { '\'' }),
        0x36 => Some(if shift { '<' } else { ',' }),
        0x37 => Some(if shift { '>' } else { '.' }),
        0x38 => Some(if shift { '?' } else { '/' }),
        _ => None,
    }
}

fn ring_doorbell(slot_id: u8, target: u32) {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let state = state_lock.lock();
        unsafe {
            if let Some(controller) = Xhci::new(state.info) {
                controller.doorbells().ring(slot_id as usize, target);
            }
        }
    }
}

fn handle_event(state: &mut ControllerState, trb_type: u8, trb: &Trb) {
    match trb_type {
        TRB_TYPE_COMMAND_COMPLETION => {
            let completion_code = ((trb.status >> 24) & 0xFF) as u8;
            let slot_id = (trb.parameter & 0xFF) as u8;
            state.last_completion_code = Some(completion_code);
            state.last_completed_slot = Some(slot_id);
            serial::write_fmt(format_args!(
                "[xhci] command completion code={:#x} slot={}\r\n",
                completion_code, slot_id
            ));
        }
        TRB_TYPE_TRANSFER_EVENT => {
            let completion_code = ((trb.status >> 24) & 0xFF) as u8;
            let trb_len = trb.status & 0x00FF_FFFF;
            let ep_id = ((trb.control >> 16) & 0x1F) as u8;
            state.last_transfer_code = Some(completion_code);
            state.last_transfer_len = Some(trb_len);
            state.last_transfer_ep = Some(ep_id);
            state.last_transfer_slot = state.active_slot; // best effort
            serial::write_fmt(format_args!(
                "[xhci] transfer event ep={} code={:#x} len={} param={:#x}\r\n",
                ep_id, completion_code, trb_len, trb.parameter
            ));

            // If it's our interrupt endpoint and success, decode and re-post
            if completion_code == 1
                && state.intr_ring_len > 0
                && ep_id == state.intr_ep_id
                && state.hid_buf_phys != 0
            {
                let len = (trb_len as usize).min(state.hid_buf_len);
                // Decode current buffer
                super::decode_hid_report(state.hid_buf_phys, len);
                // Re-post new normal TRB on interrupt ring
                let cycle = if state.intr_cycle { 1 } else { 0 };
                let trb = Trb {
                    parameter: state.hid_buf_phys,
                    status: state.hid_buf_len as u32,
                    control: ((TRB_TYPE_NORMAL & 0x3F) << 10) | (1 << 5) | cycle,
                };
                let usable = state.intr_ring_len.saturating_sub(1);
                if usable > 0 {
                    let idx = state.intr_enqueue % usable;
                    unsafe {
                        let ring = phys_to_slice_mut::<Trb>(state.intr_ring_phys, state.intr_ring_len);
                        ring[idx] = trb;
                    }
                    state.intr_enqueue = (state.intr_enqueue + 1) % usable;
                    if state.intr_enqueue == 0 {
                        state.intr_cycle = !state.intr_cycle;
                    }
                    if let Some(slot) = state.active_slot {
                        ring_doorbell(slot, state.intr_ep_id as u32);
                    }
                }
            }
        }
        TRB_TYPE_PORT_STATUS_CHANGE => {
            let port_id = ((trb.parameter >> 24) & 0xFF) as u8;
            serial::write_fmt(format_args!(
                "[xhci] port status change: port={} status={:#x}\r\n",
                port_id, trb.status
            ));
            unsafe {
                if let Some(controller) = Xhci::new(state.info) {
                    let op = controller.operational();
                    let regs = op.port((port_id.saturating_sub(1)) as usize);
                    let sc = regs.portsc();
                    let ccs = (sc & 0x1) != 0;
                    let ped = (sc & 0x2) != 0;
                    let speed = (sc >> 10) & 0xF;
                    let pls = (sc >> 5) & 0xF;
                    serial::write_fmt(format_args!(
                        "[xhci] port{} sc={:#010x} ccs={} ped={} speed={} pls={}\r\n",
                        port_id, sc, ccs as u8, ped as u8, speed, pls
                    ));
                }
            }
        }
        _ => serial::write_fmt(format_args!(
            "[xhci] event type={} status={:#x} param={:#x}\r\n",
            trb_type, trb.status, trb.parameter
        )),
    }
}

pub struct PortRegs {
    base: NonNull<u8>,
}

impl PortRegs {
    unsafe fn new(base: NonNull<u8>, offset: usize) -> Self {
        Self {
            base: NonNull::new_unchecked(base.as_ptr().add(offset)),
        }
    }

    fn sc_ptr(&self) -> *mut u32 {
        self.base.as_ptr() as *mut u32
    }

    pub fn portsc(&self) -> u32 {
        unsafe { read_volatile(self.sc_ptr()) }
    }

    #[allow(dead_code)]
    pub fn write_portsc(&self, value: u32) {
        unsafe { write_volatile(self.sc_ptr(), value) };
    }
}

fn wait_for(mut predicate: impl FnMut() -> bool) -> bool {
    for _ in 0..1_000_000 {
        if predicate() {
            return true;
        }
    }
    false
}

pub fn find_first_connected_port() -> Option<usize> {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let info = { state_lock.lock().info };
        unsafe {
            if let Some(controller) = Xhci::new(info) {
                let op = controller.operational();
                for i in 0..info.max_ports() as usize {
                    let sc = op.port(i).portsc();
                    if (sc & 0x1) != 0 {
                        return Some(i);
                    }
                }
            }
        }
    }
    None
}

pub fn reset_port(index: usize) -> bool {
    if let Some(state_lock) = CONTROLLER_STATE.get() {
        let info = { state_lock.lock().info };
        unsafe {
            if let Some(controller) = Xhci::new(info) {
                let op = controller.operational();
                let regs = op.port(index);
                let mut sc = regs.portsc();
                serial::write_fmt(format_args!("[xhci] resetting port{} sc={:#x}\r\n", index + 1, sc));
                regs.write_portsc(sc | (1 << 4));
                let _ = wait_for(|| {
                    let now = regs.portsc();
                    (now & (1 << 4)) == 0
                });
                let ok = wait_for(|| {
                    let now = regs.portsc();
                    (now & 0x2) != 0
                });
                let final_sc = regs.portsc();
                serial::write_fmt(format_args!(
                    "[xhci] port{} reset done ok={} sc={:#x}\r\n",
                    index + 1,
                    ok as u8,
                    final_sc
                ));
                return ok;
            }
        }
    }
    false
}

pub fn ensure_first_port_enabled() -> bool {
    if let Some(idx) = find_first_connected_port() {
        unsafe {
            if let Some(state_lock) = CONTROLLER_STATE.get() {
                let info = { state_lock.lock().info };
                if let Some(controller) = Xhci::new(info) {
                    let op = controller.operational();
                    let sc = op.port(idx).portsc();
                    if (sc & 0x2) != 0 {
                        return true;
                    }
                }
            }
        }
        return reset_port(idx);
    }
    false
}
