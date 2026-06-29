#![no_std]
#![no_main]

use core::arch::global_asm;

global_asm!(
    r#"
    .global _start
_start:
    xor rbp, rbp
    and rsp, -16
    call i8042_driver_main
1:
    hlt
    jmp 1b
"#
);

#[unsafe(no_mangle)]
pub extern "C" fn i8042_driver_main() -> ! {
    i8042_driver::run()
}
