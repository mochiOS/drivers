#![no_std]

use mochi_user_platform as platform;

pub fn run() -> ! {
    platform::println!("ps2-keyboard: start");

    loop {
        match platform::input::keyboard_read_tap() {
            Ok(scancode) => {
                platform::println!("ps2-keyboard: scancode=0x{:02x}", scancode);
            }
            Err(_) => platform::thread::yield_now(),
        }
    }
}
