#![no_std]

use mochi_user_platform as platform;

fn sign_extend(delta: u8, sign: bool) -> i16 {
    if sign {
        (delta as i16) - 256
    } else {
        delta as i16
    }
}

pub fn run() -> ! {
    platform::println!("ps2-mouse: start");
    let mut last_buttons = 0u8;

    loop {
        match platform::input::mouse_read() {
            Ok(packet) => {
                let b0 = (packet & 0xff) as u8;
                let b1 = ((packet >> 8) & 0xff) as u8;
                let b2 = ((packet >> 16) & 0xff) as u8;
                let buttons = b0 & 0x07;
                let x = sign_extend(b1, (b0 & 0x10) != 0);
                let dy = -sign_extend(b2, (b0 & 0x20) != 0);
                if x != 0 || dy != 0 {
                    platform::println!("ps2-mouse: moved dx={} dy={}", x, dy);
                }
                if buttons != last_buttons {
                    platform::println!(
                        "ps2-mouse: buttons 0x{:02x} -> 0x{:02x}",
                        last_buttons,
                        buttons
                    );
                    last_buttons = buttons;
                }
            }
            Err(_) => platform::thread::yield_now(),
        }
    }
}
