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
    let mut packet = [0u8; 3];
    let mut packet_len = 0usize;

    loop {
        match platform::port::in_u8(0x64) {
            Ok(status) if (status & 0x01) != 0 && (status & 0x20) != 0 => {
                let Ok(byte) = platform::port::in_u8(0x60) else {
                    platform::thread::yield_now();
                    continue;
                };

                if packet_len == 0 && (byte & 0x08) == 0 {
                    continue;
                }

                packet[packet_len] = byte;
                packet_len += 1;
                if packet_len < 3 {
                    continue;
                }
                packet_len = 0;

                let b0 = packet[0];
                let b1 = packet[1];
                let b2 = packet[2];
                if (b0 & 0xC0) != 0 {
                    continue;
                }

                let buttons = b0 & 0x07;
                let _x = sign_extend(b1, (b0 & 0x10) != 0);
                let _dy = -sign_extend(b2, (b0 & 0x20) != 0);
                if buttons != last_buttons {
                    last_buttons = buttons;
                }
            }
            _ => platform::thread::yield_now(),
        }
    }
}
