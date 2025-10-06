%define STAGE2_LOAD_ADDR        0x00080000

%ifndef KERNEL_LBA
%define KERNEL_LBA              0x00000040
%endif

BITS 32
ORG STAGE2_LOAD_ADDR

%define STACK_TOP32             0x0009F000
%define STACK_TOP64             0x0009E000
%define KERNEL_BUFFER           0x00200000
%define INITIAL_KERNEL_SECTORS  64
%define MAX_KERNEL_SECTORS      1024
%define GDT32_CODE              0x08
%define GDT32_DATA              0x10
%define GDT64_CODE              0x18
%define GDT64_DATA              0x20
%define E820_COUNT_PTR          0x00000500
%define E820_BUFFER_PTR         0x00000510
%define E820_ENTRY_SIZE         24
%define E820_MAX_ENTRIES        64

section .text

stage2_start:
    cli
    cld
    mov esp, STACK_TOP32

    call load_kernel
    call setup_gdt
    call build_page_tables
    call load_memory_map

    mov eax, cr4
    or eax, (1 << 5) | (1 << 7) | (1 << 9) | (1 << 10)
    mov cr4, eax

    mov eax, pml4
    mov cr3, eax

    mov ecx, 0xC0000080
    rdmsr
    or eax, (1 << 8)
    wrmsr

    mov eax, cr0
    or eax, (1 << 31) | (1 << 1) | (1 << 5)
    and eax, ~(1 << 2)
    mov cr0, eax

    jmp GDT64_CODE:long_mode_entry

load_kernel:
    pusha

    mov ax, GDT32_DATA
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    mov edi, KERNEL_BUFFER
    mov eax, KERNEL_LBA
    mov ecx, INITIAL_KERNEL_SECTORS
    call ata_read_lba28
    jc load_fail

    mov dword [kernel_loaded_sectors], INITIAL_KERNEL_SECTORS

    mov esi, KERNEL_BUFFER
    cmp dword [esi], 0x464C457F
    jne load_fail_magic

    mov eax, [esi + 0x18]
    mov edx, [esi + 0x1C]
    mov [kernel_entry], eax
    mov [kernel_entry + 4], edx

    mov eax, [esi + 0x20]
    mov edx, [esi + 0x24]
    mov [ph_offset], eax
    mov [ph_offset + 4], edx

    movzx eax, word [esi + 0x36]
    mov [ph_entry_size], ax
    cmp ax, 0
    je load_fail_header

    movzx eax, word [esi + 0x38]
    mov [ph_total], ax
    cmp ax, 0
    je load_fail_header
    mov [ph_remaining], ax

    movzx eax, word [ph_entry_size]
    movzx ebx, word [ph_total]
    mul ebx
    mov ebx, [ph_offset]
    mov ecx, [ph_offset + 4]
    add eax, ebx
    adc edx, ecx
    mov [tmp_end], eax
    mov [tmp_end + 4], edx
    test edx, edx
    jnz load_fail_high

    mov eax, [tmp_end]
    mov edx, [tmp_end + 4]
    add eax, 511
    adc edx, 0
    mov ecx, 9
    shrd eax, edx, cl
    shr edx, cl
    test edx, edx
    jnz load_fail_high
    mov [kernel_required_sectors], eax

    mov edx, [kernel_loaded_sectors]
    cmp edx, eax
    jae headers_ready

    mov ecx, eax
    sub ecx, edx
    mov [kernel_remaining_sectors], ecx

    mov edi, KERNEL_BUFFER
    mov ebx, edx
    imul ebx, 512
    add edi, ebx

    mov eax, KERNEL_LBA
    add eax, edx
    mov ecx, [kernel_remaining_sectors]
    call ata_read_lba28
    jc load_fail

    mov eax, [kernel_remaining_sectors]
    add [kernel_loaded_sectors], eax

headers_ready:
    mov eax, [ph_offset + 4]
    test eax, eax
    jnz load_fail_high

    mov dword [kernel_file_end], 0
    mov dword [kernel_file_end + 4], 0

    mov eax, [ph_offset]
    add eax, KERNEL_BUFFER
    mov [ph_ptr], eax
    mov [ph_iter], eax

scan_loop:
    mov ax, [ph_remaining]
    test ax, ax
    je scan_done

    mov esi, [ph_iter]
    cmp dword [esi + 0x00], 1
    jne scan_next

    mov eax, [esi + 0x08]
    mov edx, [esi + 0x0C]
    mov [tmp_offset], eax
    mov [tmp_offset + 4], edx

    mov eax, [esi + 0x20]
    mov edx, [esi + 0x24]
    mov [tmp_filesz], eax
    mov [tmp_filesz + 4], edx

    mov eax, [tmp_offset + 4]
    or eax, [tmp_filesz + 4]
    jnz load_fail_high

    mov eax, [tmp_offset]
    mov edx, [tmp_offset + 4]
    add eax, [tmp_filesz]
    adc edx, [tmp_filesz + 4]
    mov [tmp_end], eax
    mov [tmp_end + 4], edx
    test edx, edx
    jnz load_fail_high

    mov ecx, [kernel_file_end + 4]
    cmp ecx, edx
    ja scan_next
    jb scan_update_end
    mov ecx, [kernel_file_end]
    cmp ecx, eax
    jae scan_next

scan_update_end:
    mov [kernel_file_end], eax
    mov [kernel_file_end + 4], edx

scan_next:
    movzx eax, word [ph_entry_size]
    add dword [ph_iter], eax
    mov ax, [ph_remaining]
    dec ax
    mov [ph_remaining], ax
    jmp scan_loop

scan_done:
    mov ax, [ph_total]
    mov [ph_remaining], ax
    mov eax, [ph_ptr]
    mov [ph_iter], eax

    movzx eax, word [ph_entry_size]
    movzx ebx, word [ph_total]
    mul ebx
    mov ebx, [ph_offset]
    mov ecx, [ph_offset + 4]
    add eax, ebx
    adc edx, ecx
    mov [tmp_end], eax
    mov [tmp_end + 4], edx
    test edx, edx
    jnz load_fail_high

    mov ecx, [kernel_file_end + 4]
    cmp ecx, edx
    ja scan_skip_tmp
    jb scan_take_tmp
    mov ecx, [kernel_file_end]
    cmp ecx, eax
    jae scan_skip_tmp

scan_take_tmp:
    mov [kernel_file_end], eax
    mov [kernel_file_end + 4], edx

scan_skip_tmp:
    mov eax, [kernel_file_end]
    mov edx, [kernel_file_end + 4]
    test edx, edx
    jnz load_fail_high
    cmp eax, 0
    je load_fail

    add eax, 511
    adc edx, 0
    mov ecx, 9
    shrd eax, edx, cl
    shr edx, cl
    test edx, edx
    jnz load_fail_high
    cmp eax, 0
    je load_fail
    mov [kernel_required_sectors], eax
    cmp eax, MAX_KERNEL_SECTORS
    ja load_fail_high

    mov edx, [kernel_loaded_sectors]
    cmp edx, eax
    jae have_all_kernel_data

    mov ecx, eax
    sub ecx, edx
    mov [kernel_remaining_sectors], ecx

    mov edi, KERNEL_BUFFER
    mov ebx, edx
    imul ebx, 512
    add edi, ebx

    mov eax, KERNEL_LBA
    add eax, edx
    mov ecx, [kernel_remaining_sectors]
    call ata_read_lba28
    jc load_fail

    mov eax, [kernel_remaining_sectors]
    add [kernel_loaded_sectors], eax

have_all_kernel_data:
    mov ax, [ph_total]
    mov [ph_remaining], ax
    mov eax, [ph_ptr]
    mov [ph_iter], eax

load_loop:
    mov ax, [ph_remaining]
    test ax, ax
    je ph_done

    mov esi, [ph_iter]
    cmp dword [esi + 0x00], 1
    jne load_next

    mov eax, [esi + 0x08]
    mov edx, [esi + 0x0C]
    mov [tmp_offset], eax
    mov [tmp_offset + 4], edx

    mov eax, [esi + 0x10]
    mov edx, [esi + 0x14]
    mov [tmp_dest], eax
    mov [tmp_dest + 4], edx

    mov eax, [esi + 0x20]
    mov edx, [esi + 0x24]
    mov [tmp_filesz], eax
    mov [tmp_filesz + 4], edx

    mov eax, [esi + 0x28]
    mov edx, [esi + 0x2C]
    mov [tmp_memsz], eax
    mov [tmp_memsz + 4], edx

    mov eax, [tmp_offset + 4]
    or eax, [tmp_dest + 4]
    or eax, [tmp_filesz + 4]
    or eax, [tmp_memsz + 4]
    jnz load_fail_high

    mov eax, [tmp_memsz]
    cmp eax, [tmp_filesz]
    jb load_fail_segment

    mov eax, [tmp_dest]
    add eax, [tmp_memsz]
    jc load_fail_high

    mov edi, [tmp_dest]
    mov esi, [tmp_offset]
    add esi, KERNEL_BUFFER

    mov ecx, [tmp_filesz]
    jecxz .skip_copy
    rep movsb
.skip_copy:
    mov ecx, [tmp_memsz]
    sub ecx, [tmp_filesz]
    jecxz .skip_zero
    xor eax, eax
    rep stosb
.skip_zero:

load_next:
    movzx eax, word [ph_entry_size]
    add dword [ph_iter], eax
    mov ax, [ph_remaining]
    dec ax
    mov [ph_remaining], ax
    jmp load_loop

ph_done:
    popa
    ret

load_memory_map:
    pushad
    movzx eax, word [E820_COUNT_PTR]
    cmp eax, E820_MAX_ENTRIES
    jbe .count_ok
    mov eax, E820_MAX_ENTRIES
.count_ok:
    mov [boot_info_entry_count], eax
    mov dword [boot_info_entry_count + 4], 0

    mov ecx, eax
    imul ecx, E820_ENTRY_SIZE
    test ecx, ecx
    jz .done

    mov esi, E820_BUFFER_PTR
    mov edi, boot_memory_map
    rep movsb

.done:
    popad
    ret

load_fail_high:
    mov esi, msg_fail_high
    call debug_write
    jmp load_fail

load_fail_magic:
    mov esi, msg_fail_magic
    call debug_write
    jmp load_fail

load_fail_header:
    mov esi, msg_fail_header
    call debug_write
    jmp load_fail

load_fail_segment:
    mov esi, msg_fail_segment
    call debug_write
    jmp load_fail

load_fail:
    cli
.fail_debug:
    mov esi, msg_fail
    call debug_write
.fail_loop:
    hlt
    jmp .fail_loop

setup_gdt:
    lgdt [gdt_ptr]
    mov ax, GDT32_DATA
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    ret

build_page_tables:
    pushad
    cld

    mov edi, pml4
    mov ecx, 4096 / 4
    xor eax, eax
    rep stosd

    mov edi, pdpt
    mov ecx, 4096 / 4
    xor eax, eax
    rep stosd

    mov edi, pd0
    mov ecx, 4096 / 4
    xor eax, eax
    rep stosd

    mov edi, pd1
    mov ecx, 4096 / 4
    xor eax, eax
    rep stosd

    mov edi, pd2
    mov ecx, 4096 / 4
    xor eax, eax
    rep stosd

    mov edi, pd3
    mov ecx, 4096 / 4
    xor eax, eax
    rep stosd

    mov eax, pdpt
    or eax, 0x03
    mov [pml4], eax
    mov dword [pml4 + 4], 0

    mov eax, pd0
    or eax, 0x03
    mov [pdpt], eax
    mov dword [pdpt + 4], 0

    mov eax, pd1
    or eax, 0x03
    mov [pdpt + 8], eax
    mov dword [pdpt + 12], 0

    mov eax, pd2
    or eax, 0x03
    mov [pdpt + 16], eax
    mov dword [pdpt + 20], 0

    mov eax, pd3
    or eax, 0x03
    mov [pdpt + 24], eax
    mov dword [pdpt + 28], 0

    mov eax, 0
    mov edi, pd0
    call fill_pd

    mov eax, 512
    mov edi, pd1
    call fill_pd

    mov eax, 1024
    mov edi, pd2
    call fill_pd

    mov eax, 1536
    mov edi, pd3
    call fill_pd

    popad
    ret

fill_pd:
    push ebx
    mov ebx, eax
    mov ecx, 512
.fill_loop:
    mov edx, ebx
    shl edx, 21
    mov [edi], edx
    mov dword [edi + 4], 0
    or dword [edi], 0x87
    add edi, 8
    inc ebx
    loop .fill_loop
    mov eax, ebx
    pop ebx
    ret

debug_write:
    push eax
    push edx
    push esi
.loop:
    lodsb
    test al, al
    jz .done
    mov dx, 0x00E9
    out dx, al
    jmp .loop
.done:
    pop esi
    pop edx
    pop eax
    ret

ata_read_lba28:
    push ebx
    push esi
    push ebp

    mov ebp, ecx

.chunk_loop:
    test ebp, ebp
    jz .done_success

    mov ebx, ebp
    cmp ebx, 0x80
    jbe .chunk_ready
    mov ebx, 0x80
.chunk_ready:
    mov [chunk_size], ebx
    mov esi, eax

.wait_not_busy:
    mov dx, 0x1F7
    in al, dx
    test al, 0x80
    jnz .wait_not_busy
    test al, 0x21
    jnz .read_error

    mov dx, 0x1F1
    xor al, al
    out dx, al

    mov dx, 0x1F2
    mov al, bl
    out dx, al

    mov ecx, esi
    mov dx, 0x1F3
    mov al, cl
    out dx, al

    mov dx, 0x1F4
    mov al, ch
    out dx, al

    mov ecx, esi
    shr ecx, 16
    mov dx, 0x1F5
    mov al, cl
    out dx, al

    mov ecx, esi
    shr ecx, 24
    mov al, 0xE0
    or al, cl
    mov dx, 0x1F6
    out dx, al

    mov dx, 0x1F7
    mov al, 0x20
    out dx, al

    mov esi, [chunk_size]

.sector_loop:
    mov dx, 0x1F7
.wait_drq:
    in al, dx
    test al, 0x21
    jnz .read_error
    test al, 0x08
    jz .wait_drq

    mov dx, 0x1F0
    mov ecx, 256
    rep insw

    dec esi
    jnz .sector_loop

    mov dx, 0x1F7
    in al, dx
    test al, 0x21
    jnz .read_error

    mov ebx, [chunk_size]
    add eax, ebx
    sub ebp, ebx
    jmp .chunk_loop

.done_success:
    clc
.cleanup:
    pop ebp
    pop esi
    pop ebx
    ret

.read_error:
    stc
    jmp .cleanup

section .data

msg_fail_high: db "stage2: fail_high", 0
msg_fail: db "stage2: fail", 0
msg_fail_magic: db "stage2: fail_magic", 0
msg_fail_header: db "stage2: fail_header", 0
msg_fail_segment: db "stage2: fail_segment", 0

align 8
kernel_entry:              dq 0
ph_offset:                 dq 0
kernel_file_end:           dq 0
tmp_end:                   dq 0
tmp_offset:                dq 0
tmp_dest:                  dq 0
tmp_filesz:                dq 0
tmp_memsz:                dq 0

align 4
ph_ptr:                    dd 0
ph_iter:                   dd 0
kernel_loaded_sectors:     dd 0
kernel_required_sectors:   dd 0
kernel_remaining_sectors:  dd 0
chunk_size:                dd 0

align 2
ph_entry_size:             dw 0
ph_total:                  dw 0
ph_remaining:              dw 0

align 8
boot_info:
    dq boot_memory_map
boot_info_entry_count:
    dq 0
boot_info_entry_size:
    dq E820_ENTRY_SIZE

align 8
boot_memory_map:
    times E820_ENTRY_SIZE * E820_MAX_ENTRIES db 0

align 8
gdt:
    dq 0x0000000000000000
    dq 0x00CF9A000000FFFF
    dq 0x00CF92000000FFFF
    dq 0x00209A0000000000
    dq 0x0000920000000000

gdt_ptr:
    dw gdt_end - gdt - 1
    dd gdt

gdt_end:

align 4096
pml4:   times 512 dq 0
align 4096
pdpt:   times 512 dq 0
align 4096
pd0:    times 512 dq 0
align 4096
pd1:    times 512 dq 0
align 4096
pd2:    times 512 dq 0
align 4096
pd3:    times 512 dq 0

section .text
BITS 64
long_mode_entry:
    mov ax, GDT64_DATA
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax

    mov rsp, STACK_TOP64

    lea rdi, [rel boot_info]
    mov rax, [kernel_entry]
    jmp rax
