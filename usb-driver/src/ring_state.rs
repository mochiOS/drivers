//! Single-segment xHCI rings: producer/consumer ownership changes on wrap.
#[derive(Clone, Copy)]
pub(crate) struct Cursor {
    pub next_index: usize,
    pub cycle: u32,
}
impl Cursor {
    pub const fn new() -> Self { Self { next_index: 0, cycle: 1 } }
    pub fn advance(&mut self, capacity: usize) {
        assert!(capacity > 0 && self.next_index < capacity);
        self.next_index += 1;
        if self.next_index == capacity {
            self.next_index = 0;
            self.cycle ^= 1;
        }
    }
    pub fn owns(&self, control: u32) -> bool { control & 1 == self.cycle }
}

/// Input Context has one control context before slot/endpoint contexts.
pub(crate) fn endpoint_context(context_size: usize, address: u8) -> (u32, usize) {
    let dci = u32::from(address & 15) * 2 + u32::from(address & 128 != 0);
    (dci, (dci as usize + 1) * context_size)
}

pub(crate) fn received_length(status: u32, requested: usize) -> Option<usize> {
    match status >> 24 {
        1 | 13 => requested.checked_sub((status & 0xffffff) as usize),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn producer_wraps_without_using_the_link_as_payload() {
        let mut cursor = Cursor::new();
        for n in 0..10000 {
            assert_eq!(cursor.next_index, n % 255);
            assert_eq!(cursor.cycle, 1 ^ ((n / 255) as u32 & 1));
            cursor.advance(255);
        }
    }
    #[test]
    fn consumer_changes_ownership_only_at_end_of_segment() {
        let mut cursor = Cursor::new();
        for _ in 0..256 { assert!(cursor.owns(1)); cursor.advance(256); }
        assert!(!cursor.owns(1));
        assert!(cursor.owns(0));
        for _ in 0..256 { cursor.advance(256); }
        assert!(cursor.owns(1));
    }
    #[test]
    fn interrupt_input_context_includes_the_control_context() {
        for size in [32, 64] {
            assert_eq!(endpoint_context(size, 0x81), (3, 4 * size));
            assert_eq!(endpoint_context(size, 0x82), (5, 6 * size));
            assert_eq!(endpoint_context(size, 0x01), (2, 3 * size));
        }
    }
    #[test]
    fn short_reports_have_actual_lengths_and_errors_are_not_reports() {
        assert_eq!(received_length((13 << 24) | 2, 8), Some(6));
        assert_eq!(received_length(1 << 24, 8), Some(8));
        assert_eq!(received_length((13 << 24) | 9, 8), None);
        assert_eq!(received_length(6 << 24, 8), None);
    }
}
