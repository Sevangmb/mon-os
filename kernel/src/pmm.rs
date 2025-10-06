use core::sync::atomic::{AtomicU64, Ordering};

use crate::bootinfo::BootInfo;
use crate::serial;

static NEXT_FREE: AtomicU64 = AtomicU64::new(0);
static LIMIT: AtomicU64 = AtomicU64::new(0);

const PAGE_SIZE: u64 = 4096;

pub fn init(boot: &BootInfo) {
    let mut best_base = 0u64;
    let mut best_len = 0u64;

    unsafe {
        for entry in boot.memory_map() {
            if !entry.is_usable() {
                continue;
            }
            let start = align_up(entry.base_addr, PAGE_SIZE);
            let end = align_down(entry.base_addr + entry.length, PAGE_SIZE);
            if end <= start {
                continue;
            }
            let usable = end - start;
            if usable > best_len {
                best_base = start;
                best_len = usable;
            }
        }
    }

    if best_len >= PAGE_SIZE {
        NEXT_FREE.store(best_base, Ordering::SeqCst);
        LIMIT.store(best_base + best_len, Ordering::SeqCst);
        serial::write_fmt(format_args!(
            "[pmm] using region {:#x}-{:#x}\r\n",
            best_base,
            best_base + best_len
        ));
    } else {
        serial::write_str("[pmm] no usable memory found\r\n");
    }
}

pub fn alloc_aligned(size: u64, align: u64) -> Option<u64> {
    if align == 0 || align & (align - 1) != 0 {
        return None;
    }
    let adj_size = align_up(size, PAGE_SIZE.max(align));
    loop {
        let current = NEXT_FREE.load(Ordering::SeqCst);
        let limit = LIMIT.load(Ordering::SeqCst);
        if current == 0 || current >= limit {
            return None;
        }
        let aligned = align_up(current, align);
        let end = aligned.checked_add(adj_size)?;
        if end > limit {
            return None;
        }
        if NEXT_FREE
            .compare_exchange(current, end, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Some(aligned);
        }
    }
}

#[allow(dead_code)]
pub fn alloc_page() -> Option<u64> {
    alloc_aligned(PAGE_SIZE, PAGE_SIZE)
}

fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

fn align_down(value: u64, align: u64) -> u64 {
    value & !(align - 1)
}

pub fn free_kib() -> u64 {
    let next = NEXT_FREE.load(Ordering::SeqCst);
    let limit = LIMIT.load(Ordering::SeqCst);
    if next == 0 || limit <= next { 0 } else { (limit - next) / 1024 }
}
