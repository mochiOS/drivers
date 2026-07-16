#![no_std]

use mochi_user_platform as platform;

const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_INPUT_FULL: u8 = 1 << 1;
const STATUS_AUX_DATA: u8 = 1 << 5;
const INPUT_SERVICE_NAME: &str = "input.service";
const SERIAL_COM1_DATA: u16 = 0x3F8;
const SERIAL_COM1_LINE_STATUS: u16 = 0x3FD;
const SERIAL_LINE_STATUS_DATA_READY: u8 = 1 << 0;
const SCANCODE_LEFT_SHIFT: u8 = 0x2a;

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
    write_command(0xD4) && write_data(cmd) && matches!(read_data_with_timeout(100_000), Some(0xFA))
}

fn send_keyboard_command(cmd: u8) -> bool {
    write_data(cmd) && matches!(read_data_with_timeout(100_000), Some(0xFA))
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
            // Keep both device clocks enabled and leave IRQ1/IRQ12 disabled:
            // this driver polls port 0x64/0x60 from user space instead of
            // handling PS/2 interrupts in the kernel.
            //
            // Keep controller translation enabled. QEMU's PS/2 keyboard uses
            // the normal keyboard scan set, while input.service consumes set-1
            // bytes. Relying on the controller translation is more robust than
            // trying to reprogram the emulated keyboard to set 1.
            let updated = (config & !0x33) | 0x40;
            let _ = write_command(0x60);
            let _ = write_data(updated);
        }
    }

    let _ = send_keyboard_command(0xF4);
    let _ = send_mouse_command(0xF6);
    let _ = send_mouse_command(0xF4);
}

fn input_service_tid() -> u64 {
    platform::process::find_by_name(INPUT_SERVICE_NAME)
        .ok()
        .unwrap_or(0)
}

fn send_keyboard_byte(input_tid: &mut u64, byte: u8, reply: &mut [u8]) {
    if *input_tid == 0 {
        return;
    }
    let packet = [platform::input::RAW_KIND_KEYBOARD, 0, 0, 0, byte, 0, 0, 0];
    if platform::ipc::call(*input_tid, &packet, reply).is_err() {
        *input_tid = 0;
    }
}

fn ascii_to_set1(byte: u8) -> Option<(u8, bool)> {
    let lower = match byte {
        b'A'..=b'Z' => byte + (b'a' - b'A'),
        _ => byte,
    };
    let shift_for_alpha = byte.is_ascii_uppercase();
    let alpha = match lower {
        b'a' => Some(0x1e),
        b'b' => Some(0x30),
        b'c' => Some(0x2e),
        b'd' => Some(0x20),
        b'e' => Some(0x12),
        b'f' => Some(0x21),
        b'g' => Some(0x22),
        b'h' => Some(0x23),
        b'i' => Some(0x17),
        b'j' => Some(0x24),
        b'k' => Some(0x25),
        b'l' => Some(0x26),
        b'm' => Some(0x32),
        b'n' => Some(0x31),
        b'o' => Some(0x18),
        b'p' => Some(0x19),
        b'q' => Some(0x10),
        b'r' => Some(0x13),
        b's' => Some(0x1f),
        b't' => Some(0x14),
        b'u' => Some(0x16),
        b'v' => Some(0x2f),
        b'w' => Some(0x11),
        b'x' => Some(0x2d),
        b'y' => Some(0x15),
        b'z' => Some(0x2c),
        _ => None,
    };
    if let Some(scancode) = alpha {
        return Some((scancode, shift_for_alpha));
    }

    let shifted = match byte {
        b'!' => (0x02, true),
        b'@' => (0x03, true),
        b'#' => (0x04, true),
        b'$' => (0x05, true),
        b'%' => (0x06, true),
        b'^' => (0x07, true),
        b'&' => (0x08, true),
        b'*' => (0x09, true),
        b'(' => (0x0a, true),
        b')' => (0x0b, true),
        b'_' => (0x0c, true),
        b'+' => (0x0d, true),
        b'{' => (0x1a, true),
        b'}' => (0x1b, true),
        b':' => (0x27, true),
        b'"' => (0x28, true),
        b'~' => (0x29, true),
        b'|' => (0x2b, true),
        b'<' => (0x33, true),
        b'>' => (0x34, true),
        b'?' => (0x35, true),
        _ => (0, false),
    };
    if shifted.0 != 0 {
        return Some(shifted);
    }

    Some(match byte {
        b'\r' | b'\n' => (0x1c, false),
        0x08 | 0x7f => (0x0e, false),
        b'\t' => (0x0f, false),
        b' ' => (0x39, false),
        b'1' => (0x02, false),
        b'2' => (0x03, false),
        b'3' => (0x04, false),
        b'4' => (0x05, false),
        b'5' => (0x06, false),
        b'6' => (0x07, false),
        b'7' => (0x08, false),
        b'8' => (0x09, false),
        b'9' => (0x0a, false),
        b'0' => (0x0b, false),
        b'-' => (0x0c, false),
        b'=' => (0x0d, false),
        b'[' => (0x1a, false),
        b']' => (0x1b, false),
        b';' => (0x27, false),
        b'\'' => (0x28, false),
        b'`' => (0x29, false),
        b'\\' => (0x2b, false),
        b',' => (0x33, false),
        b'.' => (0x34, false),
        b'/' => (0x35, false),
        _ => return None,
    })
}

fn send_ascii_key(input_tid: &mut u64, byte: u8, reply: &mut [u8]) {
    let Some((scancode, needs_shift)) = ascii_to_set1(byte) else {
        return;
    };
    if needs_shift {
        send_keyboard_byte(input_tid, SCANCODE_LEFT_SHIFT, reply);
    }
    send_keyboard_byte(input_tid, scancode, reply);
    send_keyboard_byte(input_tid, scancode | 0x80, reply);
    if needs_shift {
        send_keyboard_byte(input_tid, SCANCODE_LEFT_SHIFT | 0x80, reply);
    }
}

fn poll_serial_keyboard(input_tid: &mut u64, reply: &mut [u8]) {
    loop {
        let Ok(status) = platform::port::in_u8(SERIAL_COM1_LINE_STATUS) else {
            return;
        };
        if (status & SERIAL_LINE_STATUS_DATA_READY) == 0 {
            return;
        }
        let Ok(byte) = platform::port::in_u8(SERIAL_COM1_DATA) else {
            return;
        };
        send_ascii_key(input_tid, byte, reply);
        if *input_tid == 0 {
            return;
        }
    }
}

pub fn run(sp: *const usize) -> ! {
    unsafe {
        let _ = platform::logger::init_from_initial_stack(sp);
    }
    init_i8042();
    let mut input_tid = input_service_tid();
    let mut mouse_packet = [0u8; 3];
    let mut mouse_packet_len = 0usize;
    let mut reply = [];

    loop {
        if input_tid == 0 {
            input_tid = input_service_tid();
            platform::thread::yield_now();
            continue;
        }

        poll_serial_keyboard(&mut input_tid, &mut reply);
        if input_tid == 0 {
            continue;
        }

        let status = match platform::port::in_u8(0x64) {
            Ok(status) => status,
            Err(_) => {
                platform::thread::yield_now();
                continue;
            }
        };

        if (status & STATUS_OUTPUT_FULL) == 0 {
            platform::thread::yield_now();
            continue;
        }

        let byte = match platform::port::in_u8(0x60) {
            Ok(byte) => byte,
            Err(_) => {
                platform::thread::yield_now();
                continue;
            }
        };

        if (status & STATUS_AUX_DATA) == 0 {
            send_keyboard_byte(&mut input_tid, byte, &mut reply);
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

        let packet = [
            platform::input::RAW_KIND_MOUSE_PACKET,
            0,
            0,
            0,
            mouse_packet[0],
            mouse_packet[1],
            mouse_packet[2],
            0,
        ];
        if platform::ipc::call(input_tid, &packet, &mut reply).is_err() {
            input_tid = 0;
        }
    }
}
