#![no_std]

use mochi_user_platform as platform;

const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_INPUT_FULL: u8 = 1 << 1;
const STATUS_AUX_DATA: u8 = 1 << 5;
const INPUT_SERVICE_NAME: &str = "input.service";

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
            // Enable IRQ1/IRQ12 and keyboard translation so the first port
            // produces set-1 scancodes expected by input.service.
            let updated = (config | 0x43) & !0x30;
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
    unsafe {
        let _ = platform::logger::init_from_initial_stack(sp);
    }
    platform::println!("i8042: start");
    init_i8042();
    let mut input_tid = input_service_tid();
    let mut keyboard_log_budget = 8usize;
    let mut reply = [0u8; 8];

    loop {
        if input_tid == 0 {
            input_tid = input_service_tid();
            platform::thread::yield_now();
            continue;
        }

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
            if keyboard_log_budget > 0 {
                platform::println!("i8042: key byte=0x{:02x}", byte);
                keyboard_log_budget -= 1;
            }
            let packet = [platform::input::RAW_KIND_KEYBOARD, 0, 0, 0, byte, 0, 0, 0];
            if platform::ipc::call(input_tid, &packet, &mut reply).is_err() {
                input_tid = 0;
            }
            continue;
        }
    }
}
