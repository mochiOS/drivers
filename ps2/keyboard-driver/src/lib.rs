#![no_std]

use mochi_user_platform as platform;

pub fn run() -> ! {
    platform::println!("ps2-keyboard: start");

    loop {
        match platform::port::in_u8(0x64) {
            Ok(status) if (status & 0x01) != 0 && (status & 0x20) == 0 => {
                if let Ok(scancode) = platform::port::in_u8(0x60) {
                    platform::println!("ps2-keyboard: scancode=0x{:02x}", scancode);
                }
            }
            _ => platform::thread::yield_now(),
        }
    }
}
