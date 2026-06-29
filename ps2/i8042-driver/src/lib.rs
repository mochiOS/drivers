#![no_std]

use mochi_user_platform as platform;

const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_INPUT_FULL: u8 = 1 << 1;
const STATUS_AUX_DATA: u8 = 1 << 5;

fn sign_extend(delta: u8, sign: bool) -> i16 {
    if sign {
        (delta as i16) - 256
    } else {
        delta as i16
    }
}

fn wait_input_clear(mut budget: usize) -> bool {
    while budget > 0 {
        match platform::port::in_u8(0x64) {
            Ok(status) if (status & STATUS_INPUT_FULL) == 0 => return true,
            _ => {}
        }
        budget -= 1;
        core::hint::spin_loop();
    }
    false
}

fn wait_output_full(mut budget: usize) -> bool {
    while budget > 0 {
        match platform::port::in_u8(0x64) {
            Ok(status) if (status & STATUS_OUTPUT_FULL) != 0 => return true,
            _ => {}
        }
        budget -= 1;
        core::hint::spin_loop();
    }
    false
}

fn write_command(value: u8) -> bool {
    if !wait_input_clear(100_000) {
        return false;
    }
    platform::port::out_u8(0x64, value).is_ok()
}

fn write_data(value: u8) -> bool {
    if !wait_input_clear(100_000) {
        return false;
    }
    platform::port::out_u8(0x60, value).is_ok()
}

fn read_data_with_timeout(budget: usize) -> Option<u8> {
    if !wait_output_full(budget) {
        return None;
    }
    platform::port::in_u8(0x60).ok()
}

fn flush_output_buffer() {
    for _ in 0..32 {
        if !wait_output_full(1_000) {
            break;
        }
        let _ = platform::port::in_u8(0x60);
    }
}

fn send_mouse_command(cmd: u8) -> bool {
    write_command(0xD4)
        && write_data(cmd)
        && matches!(read_data_with_timeout(100_000), Some(0xFA))
}

fn init_i8042() {
    flush_output_buffer();

    let _ = write_command(0xAD);
    let _ = write_command(0xA7);
    flush_output_buffer();

    let _ = write_command(0xAE);
    let _ = write_command(0xA8);

    if write_command(0x20) {
        if let Some(config) = read_data_with_timeout(100_000) {
            let updated = (config | 0x03) & !0x30;
            let _ = write_command(0x60);
            let _ = write_data(updated);
        }
    }

    let _ = send_mouse_command(0xF6);
    let _ = send_mouse_command(0xF4);
}

fn parse_decimal_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let mut out = 0u64;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?;
        out = out.checked_add(u64::from(b - b'0'))?;
    }
    Some(out)
}

unsafe fn c_string_len(ptr: *const u8) -> usize {
    let mut len = 0usize;
    loop {
        let ch = unsafe { core::ptr::read_volatile(ptr.add(len)) };
        if ch == 0 {
            return len;
        }
        len += 1;
    }
}

unsafe fn parse_endpoint_arg(sp: *const usize) -> Option<u64> {
    let stack = unsafe { platform::runtime::InitialStack::parse(sp) };
    for &arg_ptr in stack.argv {
        if arg_ptr.is_null() {
            continue;
        }
        let len = unsafe { c_string_len(arg_ptr) };
        let arg = unsafe { core::slice::from_raw_parts(arg_ptr, len) };
        if let Some(value) = parse_decimal_u64(arg) {
            return Some(value);
        }
    }
    None
}

pub fn run(sp: *const usize) -> ! {
    platform::println!("i8042: start");
    init_i8042();
    let input_endpoint = unsafe { parse_endpoint_arg(sp) }.unwrap_or(0);

    let mut mouse_buttons = 0u8;
    let mut mouse_packet = [0u8; 3];
    let mut mouse_packet_len = 0usize;

    loop {
        let Ok(status) = platform::port::in_u8(0x64) else {
            platform::thread::yield_now();
            continue;
        };

        if (status & STATUS_OUTPUT_FULL) == 0 {
            platform::thread::yield_now();
            continue;
        }

        let Ok(byte) = platform::port::in_u8(0x60) else {
            platform::thread::yield_now();
            continue;
        };

        if (status & STATUS_AUX_DATA) == 0 {
            if input_endpoint != 0 {
                let _ = platform::ipc::send(input_endpoint, &[byte]);
            }
            continue;
        }

        if mouse_packet_len == 0 && (byte & 0x08) == 0 {
            continue;
        }
        mouse_packet[mouse_packet_len] = byte;
        mouse_packet_len += 1;
        if mouse_packet_len < 3 {
            continue;
        }
        mouse_packet_len = 0;

        let b0 = mouse_packet[0];
        let b1 = mouse_packet[1];
        let b2 = mouse_packet[2];
        if (b0 & 0xC0) != 0 {
            continue;
        }

        let buttons = b0 & 0x07;
        let x = sign_extend(b1, (b0 & 0x10) != 0);
        let dy = -sign_extend(b2, (b0 & 0x20) != 0);
        if x != 0 || dy != 0 {
            platform::println!("i8042: mouse moved dx={} dy={}", x, dy);
        }
        if buttons != mouse_buttons {
            platform::println!(
                "i8042: mouse buttons 0x{:02x} -> 0x{:02x}",
                mouse_buttons,
                buttons
            );
            mouse_buttons = buttons;
        }
    }
}
