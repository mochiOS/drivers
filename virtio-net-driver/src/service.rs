use crate::device::{NetDevice, NetError};
use mochi_user_platform as platform;
use mochios_net_device_protocol::{
    HEADER_LEN, Header, INTERFACE_INFO_LEN, MAX_FRAME_LEN, Opcode, STATISTICS_LEN, STATUS_LEN,
    decode_empty, decode_frame, encode_frame, encode_interface_info, encode_statistics,
    encode_status,
};
const REQUEST_BUFFER_LEN: usize = HEADER_LEN + 4 + MAX_FRAME_LEN;
const REPLY_BUFFER_LEN: usize = REQUEST_BUFFER_LEN;
pub(crate) fn run() -> ! {
    platform::println!("virtio-net.driver: start");
    let mut device = match NetDevice::initialize() {
        Ok(d) => d,
        Err(e) => {
            platform::println!(
                "virtio-net.driver: initialization failed error={:?} errno={}",
                e,
                e.errno()
            );
            idle()
        }
    };
    let info = device.info();
    platform::println!(
        "virtio-net.driver: ready mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} link={} mtu={}",
        info.mac[0],
        info.mac[1],
        info.mac[2],
        info.mac[3],
        info.mac[4],
        info.mac[5],
        info.link_up,
        info.mtu
    );
    let mut request = [0u8; REQUEST_BUFFER_LEN];
    let mut reply = [0u8; REPLY_BUFFER_LEN];
    loop {
        if let Err(error) = device.poll() {
            platform::println!(
                "virtio-net.driver: queue error={:?} errno={}",
                error,
                error.errno()
            );
            device.fail();
            idle()
        }
        match platform::ipc::try_wait(&mut request) {
            Ok(message) => {
                let len = (message & 0xffff_ffff) as usize;
                let sender = message >> 32;
                let Some(bytes) = request.get(..len) else {
                    continue;
                };
                let reply_len = handle(&mut device, bytes, &mut reply);
                if let Some(n) = reply_len {
                    let _ = platform::ipc::reply(sender, &reply[..n]);
                }
            }
            Err(error) if error.raw() == mochi_user_syscall::EAGAIN as i64 => {
                platform::thread::yield_now()
            }
            Err(_) => platform::thread::yield_now(),
        }
    }
}
fn handle(device: &mut NetDevice, request: &[u8], reply: &mut [u8]) -> Option<usize> {
    let header = Header::decode(request).ok()?;
    match header.opcode {
        Opcode::GetInterfaceInfo => {
            decode_empty(Opcode::GetInterfaceInfo, request).ok()?;
            encode_interface_info(
                header.request_id,
                device.info(),
                &mut reply[..INTERFACE_INFO_LEN],
            )
            .ok()
        }
        Opcode::TransmitFrame => {
            let (id, frame) = decode_frame(Opcode::TransmitFrame, request).ok()?;
            let status = match device.transmit(frame) {
                Ok(()) => 0,
                Err(NetError::QueueExhausted) => -(mochi_user_syscall::EAGAIN as i32),
                Err(_) => -(mochi_user_syscall::EIO as i32),
            };
            encode_status(
                Opcode::TransmitComplete,
                id,
                status,
                &mut reply[..STATUS_LEN],
            )
            .ok()
        }
        Opcode::ReceiveFrame => {
            decode_empty(Opcode::ReceiveFrame, request).ok()?;
            match device.receive() {
                Some(frame) => {
                    encode_frame(Opcode::FrameReceived, header.request_id, &frame, reply).ok()
                }
                None => encode_status(
                    Opcode::FrameReceived,
                    header.request_id,
                    -(mochi_user_syscall::EAGAIN as i32),
                    &mut reply[..STATUS_LEN],
                )
                .ok(),
            }
        }
        Opcode::GetStatistics => {
            decode_empty(Opcode::GetStatistics, request).ok()?;
            encode_statistics(
                Opcode::Statistics,
                header.request_id,
                device.statistics(),
                &mut reply[..STATISTICS_LEN],
            )
            .ok()
        }
        _ => None,
    }
}
fn idle() -> ! {
    loop {
        platform::thread::yield_now()
    }
}
