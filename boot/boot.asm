; Minimal bootsector: loads stage2, enters protected mode
BITS 16
ORG 0x7C00

%ifndef STAGE2_LBA
%define STAGE2_LBA         0x00000001
%endif

%ifndef STAGE2_SECTORS
%define STAGE2_SECTORS     64
%endif

%ifndef STAGE2_LOAD_ADDR
%define STAGE2_LOAD_ADDR   0x00080000
%endif

%ifndef CODE32_SEL
%define CODE32_SEL         0x08
%endif

%ifndef DATA32_SEL
%define DATA32_SEL         0x10
%endif

%define E820_COUNT_PTR     0x00000500
%define E820_BUFFER_PTR    0x00000510
%define E820_ENTRY_SIZE    24
%define E820_MAX_ENTRIES   64

start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00

    mov [boot_drive], dl
    mov si, boot_msg
    call print_string

    call detect_memory

    call enable_a20
    call load_stage2

    lgdt [gdt_ptr]

    mov eax, cr0
    or eax, 0x00000001
    mov cr0, eax

    jmp CODE32_SEL:prot_entry

print_string:
    mov ah, 0x0E
.next_char:
    lodsb
    cmp al, 0
    je .done
    int 0x10
    jmp .next_char
.done:
    ret

enable_a20:
    in al, 0x92
    or al, 0x02
    and al, 0xFE
    out 0x92, al
    ret

detect_memory:
    pushad
    cld

    mov di, E820_BUFFER_PTR
    mov cx, E820_ENTRY_SIZE * E820_MAX_ENTRIES
    xor ax, ax
    rep stosb

    mov di, E820_BUFFER_PTR
    mov word [E820_COUNT_PTR], 0
    xor ebx, ebx

.next_entry:
    mov eax, 0xE820
    mov edx, 0x534D4150
    mov ecx, E820_ENTRY_SIZE
    int 0x15
    jc .done
    cmp eax, 0x534D4150
    jne .done
    cmp ecx, 20
    jb .done

    mov ax, [E820_COUNT_PTR]
    cmp ax, E820_MAX_ENTRIES
    jae .done
    inc ax
    mov [E820_COUNT_PTR], ax

    test ebx, ebx
    jz .done

    add di, E820_ENTRY_SIZE
    jmp .next_entry

.done:
    popad
    ret

load_stage2:
    mov si, dap
    mov ah, 0x42
    mov dl, [boot_drive]
    int 0x13
    jc disk_error
    ret

disk_error:
    mov si, err_msg
    call print_string
.hang:
    hlt
    jmp .hang

prot_entry:
BITS 32
    mov ax, DATA32_SEL
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax
    mov esp, 0x0009FC00

    jmp dword STAGE2_LOAD_ADDR

boot_msg: db "Booting...", 0x0D, 0x0A, 0
err_msg:  db "Disk error", 0
boot_drive: db 0

ALIGN 16
dap:
    db 0x10
    db 0x00
    dw STAGE2_SECTORS
    dw STAGE2_LOAD_ADDR & 0x0F
    dw (STAGE2_LOAD_ADDR >> 4) & 0xFFFF
    dq STAGE2_LBA

ALIGN 8
gdt:
    dq 0x0000000000000000
    dq 0x00CF9A000000FFFF
    dq 0x00CF92000000FFFF

gdt_ptr:
    dw gdt_end - gdt - 1
    dd gdt

gdt_end:

times 510-($-$$) db 0
    dw 0xAA55
