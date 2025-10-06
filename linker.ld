OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

PHDRS
{
  text PT_LOAD FLAGS(5);
  data PT_LOAD FLAGS(6);
}

SECTIONS
{
  . = 0x00100000;

  .text ALIGN(0x1000) :
  {
    KEEP(*(.text._start))
    KEEP(*(.text._start.*))
    *(.text*)
  } :text

  .rodata ALIGN(0x1000) :
  {
    *(.rodata*)
  } :text

  .data ALIGN(0x1000) :
  {
    *(.data*)
  } :data

  .bss ALIGN(0x1000) (NOLOAD) :
  {
    *(.bss*)
    *(COMMON)
  } :data
}
