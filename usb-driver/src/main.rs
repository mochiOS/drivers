#![no_std]
#![no_main]

use core::arch::global_asm;

global_asm!(
    r#"
    .global _start
_start:
    xor rbp, rbp
    mov rdi, rsp
    and rsp, -16
    call usb_driver_main
1:
    hlt
    jmp 1b
"#
);

/// Receive the native launch stack and install the inherited logger endpoint.
#[unsafe(no_mangle)]
pub extern "C" fn usb_driver_main(sp: *const usize) -> ! {
    // SAFETY: _start preserves the kernel-provided initial stack before aligning it.
    unsafe {
        let _ = mochi_user_platform::logger::init_from_initial_stack(sp);
    }
    usb_driver::run()
}
