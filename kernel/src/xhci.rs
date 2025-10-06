use crate::pmm;
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
const TRB_TYPE_NO_OP_COMMAND: u32 = 23;

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
