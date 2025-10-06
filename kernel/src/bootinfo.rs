use core::{cmp::max, mem};

#[repr(C)]
pub struct BootInfo {
    pub memory_map: *const MemoryMapEntry,
    pub memory_map_len: u64,
    pub memory_map_entry_size: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MemoryMapEntry {
    pub base_addr: u64,
    pub length: u64,
    pub region_type: u32,
    pub attributes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryRegionKind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    Unknown(u32),
}

impl MemoryRegionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryRegionKind::Usable => "Usable",
            MemoryRegionKind::Reserved => "Reserved",
            MemoryRegionKind::AcpiReclaimable => "ACPI Reclaimable",
            MemoryRegionKind::AcpiNvs => "ACPI NVS",
            MemoryRegionKind::BadMemory => "Bad Memory",
            MemoryRegionKind::Unknown(_) => "Unknown",
        }
    }
}

impl MemoryMapEntry {
    pub fn kind(&self) -> MemoryRegionKind {
        match self.region_type {
            1 => MemoryRegionKind::Usable,
            2 => MemoryRegionKind::Reserved,
            3 => MemoryRegionKind::AcpiReclaimable,
            4 => MemoryRegionKind::AcpiNvs,
            5 => MemoryRegionKind::BadMemory,
            other => MemoryRegionKind::Unknown(other),
        }
    }

    pub fn is_usable(&self) -> bool {
        self.region_type == 1
    }
}

pub struct MemoryMapIter {
    ptr: *const u8,
    remaining: u64,
    stride: usize,
}

impl BootInfo {
    pub unsafe fn memory_map(&self) -> MemoryMapIter {
        let stride = max(
            self.memory_map_entry_size,
            mem::size_of::<MemoryMapEntry>() as u64,
        ) as usize;
        MemoryMapIter {
            ptr: self.memory_map.cast(),
            remaining: self.memory_map_len,
            stride,
        }
    }
}

impl Iterator for MemoryMapIter {
    type Item = &'static MemoryMapEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let entry = unsafe { &*(self.ptr as *const MemoryMapEntry) };
        self.ptr = unsafe { self.ptr.add(self.stride) };
        self.remaining -= 1;
        Some(entry)
    }
}
