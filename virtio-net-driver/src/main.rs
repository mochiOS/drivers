#![no_std]
#![no_main]

extern crate alloc;

use core::arch::global_asm;

mod device;
mod dma;
mod pci;
mod service;

global_asm!(
    r#"
    .global _start
_start:
    xor rbp, rbp
    mov rdi, rsp
    and rsp, -16
    call virtio_net_main
1:
    hlt
    jmp 1b
"#
);

#[unsafe(no_mangle)]
pub extern "C" fn virtio_net_main(sp: *const usize) -> ! {
    unsafe {
        let _ = mochi_user_platform::logger::init_from_initial_stack(sp);
    }
    service::run()
}
