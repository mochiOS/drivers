#![no_std]

use core::ops::Range;

pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;
pub const VIRTIO_NET_F_MAC: u64 = 1 << 5;
pub const VIRTIO_NET_F_STATUS: u64 = 1 << 16;
pub const REQUIRED_FEATURES: u64 = VIRTIO_F_VERSION_1 | VIRTIO_NET_F_MAC;
pub const REQUESTED_FEATURES: u64 = REQUIRED_FEATURES | VIRTIO_NET_F_STATUS;
pub const VIRTIO_NET_HEADER_LEN: usize = 12;
pub const ETHERNET_HEADER_LEN: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameBoundsError {
    TooShort,
    TooLong,
}

pub const fn selected_features(offered: u64) -> Option<u64> {
    if offered & REQUIRED_FEATURES != REQUIRED_FEATURES {
        return None;
    }
    Some(offered & REQUESTED_FEATURES)
}

pub fn received_frame_range(
    written: usize,
    buffer_len: usize,
) -> Result<Range<usize>, FrameBoundsError> {
    if written < VIRTIO_NET_HEADER_LEN + ETHERNET_HEADER_LEN {
        return Err(FrameBoundsError::TooShort);
    }
    if written > buffer_len {
        return Err(FrameBoundsError::TooLong);
    }
    Ok(VIRTIO_NET_HEADER_LEN..written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_implemented_features() {
        let unsupported = 1 | (1 << 7) | (1 << 15) | (1 << 17) | (1 << 22);
        assert_eq!(
            selected_features(REQUESTED_FEATURES | unsupported),
            Some(REQUESTED_FEATURES)
        );
        assert_eq!(selected_features(VIRTIO_F_VERSION_1), None);
        assert_eq!(selected_features(VIRTIO_NET_F_MAC), None);
    }

    #[test]
    fn modern_header_and_short_frames_are_fixed() {
        assert_eq!(VIRTIO_NET_HEADER_LEN, 12);
        assert_eq!(
            received_frame_range(25, 1_526),
            Err(FrameBoundsError::TooShort)
        );
        assert_eq!(received_frame_range(26, 1_526), Ok(12..26));
        assert_eq!(
            received_frame_range(1_527, 1_526),
            Err(FrameBoundsError::TooLong)
        );
    }
}
