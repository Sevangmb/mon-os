use core::fmt;
use x86_64::instructions::port::Port;

#[derive(Clone, Copy)]
pub struct PciAddress {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl fmt::Display for PciAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x}:{:02x}.{}", self.bus, self.device, self.function)
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct Bar {
    pub base: u64,
    pub is_memory: bool,
    pub is_64bit: bool,
    pub prefetchable: bool,
}

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

pub fn read_u32(addr: PciAddress, offset: u8) -> u32 {
    let aligned_offset = offset & !0x03;
    let function = addr.function as u32;
    let device = addr.device as u32;
    let bus = addr.bus as u32;
    let address =
        (1u32 << 31) | (bus << 16) | (device << 11) | (function << 8) | aligned_offset as u32;

    unsafe {
        let mut addr_port = Port::<u32>::new(CONFIG_ADDRESS);
        let mut data_port = Port::<u32>::new(CONFIG_DATA);
        addr_port.write(address);
        data_port.read()
    }
}

pub fn read_u16(addr: PciAddress, offset: u8) -> u16 {
    let value = read_u32(addr, offset);
    let shift = (offset & 0x02) * 8;
    ((value >> shift) & 0xFFFF) as u16
}

pub fn read_u8(addr: PciAddress, offset: u8) -> u8 {
    let value = read_u32(addr, offset);
    let shift = (offset & 0x03) * 8;
    ((value >> shift) & 0xFF) as u8
}

pub fn vendor_id(addr: PciAddress) -> u16 {
    read_u16(addr, 0x00)
}

pub fn device_id(addr: PciAddress) -> u16 {
    read_u16(addr, 0x02)
}

pub fn class_code(addr: PciAddress) -> u8 {
    read_u8(addr, 0x0B)
}

pub fn subclass(addr: PciAddress) -> u8 {
    read_u8(addr, 0x0A)
}

pub fn prog_if(addr: PciAddress) -> u8 {
    read_u8(addr, 0x09)
}

pub fn header_type(addr: PciAddress) -> u8 {
    read_u8(addr, 0x0E)
}

pub fn has_function(addr: PciAddress) -> bool {
    vendor_id(addr) != 0xFFFF
}

pub fn enumerate<F>(mut callback: F)
where
    F: FnMut(PciAddress),
{
    for bus in 0u8..=255 {
        for device in 0u8..32 {
            let addr = PciAddress {
                bus,
                device,
                function: 0,
            };
            if !has_function(addr) {
                continue;
            }

            callback(addr);

            let header = header_type(addr);
            if header & 0x80 != 0 {
                for function in 1u8..8 {
                    let addr = PciAddress {
                        bus,
                        device,
                        function,
                    };
                    if has_function(addr) {
                        callback(addr);
                    }
                }
            }
        }
    }
}

pub fn find_usb_controllers(mut callback: impl FnMut(PciAddress)) {
    enumerate(|addr| {
        let class = class_code(addr);
        let subclass = subclass(addr);
        if class == 0x0C && subclass == 0x03 {
            callback(addr);
        }
    });
}

pub fn bar(addr: PciAddress, index: u8) -> Option<Bar> {
    if index >= 6 {
        return None;
    }

    let offset = 0x10u8 + index * 4;
    let value = read_u32(addr, offset);
    if value == 0 {
        return None;
    }

    if value & 0x01 != 0 {
        // I/O space BAR
        let base = (value & !0x3) as u64;
        return Some(Bar {
            base,
            is_memory: false,
            is_64bit: false,
            prefetchable: false,
        });
    }

    let bar_type = (value >> 1) & 0x3;
    let prefetchable = (value & (1 << 3)) != 0;
    let mut base = (value & 0xFFFF_FFF0) as u64;
    let is_64bit = bar_type == 0x2;

    if is_64bit {
        let high = read_u32(addr, offset + 4);
        base |= (high as u64) << 32;
    }

    Some(Bar {
        base,
        is_memory: true,
        is_64bit,
        prefetchable,
    })
}
