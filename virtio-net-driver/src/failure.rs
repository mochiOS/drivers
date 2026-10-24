//! A failed device remains a responsive IPC endpoint. Never leave a valid
//! caller parked in a synchronous request merely because initialization failed.
use mochios_net_device_protocol::{Header, Opcode, decode_empty, decode_frame, encode_status};

pub(crate) fn reply(status: i32, request: &[u8], response: &mut [u8]) -> Option<usize> {
    let header = Header::decode(request).ok()?;
    let opcode = match header.opcode {
        Opcode::GetInterfaceInfo => {
            decode_empty(header.opcode, request).ok()?;
            Opcode::InterfaceInfo
        }
        Opcode::GetStatistics => {
            decode_empty(header.opcode, request).ok()?;
            Opcode::Statistics
        }
        Opcode::ReceiveFrame => {
            decode_empty(header.opcode, request).ok()?;
            Opcode::FrameReceived
        }
        Opcode::TransmitFrame => {
            decode_frame(header.opcode, request).ok()?;
            Opcode::TransmitComplete
        }
        _ => return None,
    };
    let status = if status < 0 && status != i32::MIN {
        status
    } else {
        -5
    };
    encode_status(opcode, header.request_id, status, response).ok()
}
