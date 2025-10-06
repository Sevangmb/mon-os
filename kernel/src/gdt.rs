use spin::Once;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, FS, GS, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::PrivilegeLevel;
use x86_64::VirtAddr;

const DOUBLE_FAULT_STACK_SIZE: usize = 4096;
const KERNEL_STACK_SIZE: usize = 4096 * 4;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    user_code: SegmentSelector,
    user_data: SegmentSelector,
    tss: SegmentSelector,
}

static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();
static TSS: Once<TaskStateSegment> = Once::new();

#[repr(align(16))]
struct AlignedStack<const SIZE: usize>([u8; SIZE]);

static mut DOUBLE_FAULT_STACK: AlignedStack<DOUBLE_FAULT_STACK_SIZE> =
    AlignedStack([0; DOUBLE_FAULT_STACK_SIZE]);
static mut KERNEL_STACK: AlignedStack<KERNEL_STACK_SIZE> = AlignedStack([0; KERNEL_STACK_SIZE]);

pub fn init() {
    let tss = TSS.call_once(init_tss);

    let gdt = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();
        let code = gdt.add_entry(Descriptor::kernel_code_segment());
        let data = gdt.add_entry(Descriptor::kernel_data_segment());
        let user_code_sel = gdt.add_entry(Descriptor::user_code_segment());
        let user_data_sel = gdt.add_entry(Descriptor::user_data_segment());
        let tss = gdt.add_entry(Descriptor::tss_segment(tss));
        let user_code = SegmentSelector::new(user_code_sel.index(), PrivilegeLevel::Ring3);
        let user_data = SegmentSelector::new(user_data_sel.index(), PrivilegeLevel::Ring3);
        (
            gdt,
            Selectors {
                code,
                data,
                user_code,
                user_data,
                tss,
            },
        )
    });

    gdt.0.load();
    unsafe {
        CS::set_reg(gdt.1.code);
        SS::set_reg(gdt.1.data);
        DS::set_reg(gdt.1.data);
        ES::set_reg(gdt.1.data);
        FS::set_reg(gdt.1.data);
        GS::set_reg(gdt.1.data);
        load_tss(gdt.1.tss);
    }
}

fn init_tss() -> TaskStateSegment {
    let mut tss = TaskStateSegment::new();
    let df_stack_start =
        unsafe { VirtAddr::from_ptr(core::ptr::addr_of!(DOUBLE_FAULT_STACK.0) as *const u8) };
    let df_stack_end = df_stack_start + DOUBLE_FAULT_STACK_SIZE as u64;
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = df_stack_end;

    let kernel_stack_start =
        unsafe { VirtAddr::from_ptr(core::ptr::addr_of!(KERNEL_STACK.0) as *const u8) };
    let kernel_stack_end = kernel_stack_start + KERNEL_STACK_SIZE as u64;
    tss.privilege_stack_table[0] = kernel_stack_end;
    tss
}

#[allow(dead_code)]
pub fn selectors() -> Selectors {
    GDT.get().expect("GDT not initialized").1
}

#[allow(dead_code)]
pub fn kernel_stack_top() -> VirtAddr {
    selectors();
    let start = unsafe { VirtAddr::from_ptr(core::ptr::addr_of!(KERNEL_STACK.0) as *const u8) };
    start + KERNEL_STACK_SIZE as u64
}
