// Import initrd symbols from the global linkage (defined in ai_link.rs)
extern "C" {
    static mut INITRD_BASE: *const u8;
    static mut INITRD_LEN: usize;
}

pub struct Entry<'a> {
    pub name: &'a [u8],
    pub data: *const u8,
    pub size: usize,
}

pub fn for_each(mut f: impl FnMut(Entry)) {
    unsafe {
        let base = INITRD_BASE;
        let len = INITRD_LEN;
        if base.is_null() || len < 110 { return; }
        // Verify header magic 'AIRD' + length at header sector
        // stage2 places 'AIRD'+len at sector before initrd data; INITRD_BASE points to data start.
        // We trust INITRD_BASE here and parse cpio newc at that address.
        let mut off: usize = 0;
        while off + 110 <= len {
            let magic = core::slice::from_raw_parts(base.add(off), 6);
            if magic != b"070701" { break; }
            let read_hex = |fo: usize| -> Option<u32> {
                let s = core::slice::from_raw_parts(base.add(off + fo), 8);
                let mut v = 0u32;
                for &c in s { let d = match c { b'0'..=b'9'=>c-b'0', b'a'..=b'f'=>10+(c-b'a'), b'A'..=b'F'=>10+(c-b'A'), _=>return None }; v = (v<<4)|(d as u32);} Some(v)
            };
            let namesize = match read_hex(94) { Some(x)=>x as usize, None=>break };
            let filesize = match read_hex(54) { Some(x)=>x as usize, None=>break };
            let name_off = off + 110;
            if name_off + namesize > len { break; }
            let name_bytes = core::slice::from_raw_parts(base.add(name_off), namesize);
            let fname = if namesize>0 { &name_bytes[..namesize-1] } else { name_bytes };
            let mut data_off = (name_off + namesize + 3) & !3;
            if fname == b"TRAILER!!!" { break; }
            if data_off + filesize > len { break; }
            f(Entry { name: fname, data: base.add(data_off), size: filesize });
            let mut next = data_off + filesize;
            next = (next + 3) & !3;
            off = next;
        }
    }
}

pub fn find(path: &str) -> Option<(*const u8, usize)> {
    let mut out: Option<(*const u8, usize)> = None;
    for_each(|e| {
        if out.is_some() { return; }
        let want = path.as_bytes();
        if e.name == want {
            out = Some((e.data, e.size));
        }
    });
    out
}
