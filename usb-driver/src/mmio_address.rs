//! Process-private virtual addresses for borrowed USB MMIO, never anonymous RAM.
use core::sync::atomic::{AtomicU64, Ordering};

const BASE: u64 = 0x0000_6100_0000_0000;
const END: u64 = 0x0000_6180_0000_0000;
// Owned only by this driver process. Kernel mapping checks reject occupied
// pages; this cursor neither creates mappings nor grants physical authority.
static NEXT: AtomicU64 = AtomicU64::new(BASE);

pub(crate) fn take(bytes: u64) -> Option<u64> {
    allocate(&NEXT, bytes)
}
fn allocate(next: &AtomicU64, bytes: u64) -> Option<u64> {
    if bytes == 0 || bytes & 4095 != 0 {
        return None;
    }
    next.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |start| {
        start.checked_add(bytes).filter(|end| *end <= END)
    })
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successive_mappings_are_disjoint_without_allocating_backing_ram() {
        let next = AtomicU64::new(BASE);
        assert_eq!(allocate(&next, 16384), Some(BASE));
        assert_eq!(allocate(&next, 4096), Some(BASE + 16384));
    }
    #[test]
    fn invalid_extents_and_exhaustion_do_not_advance_the_cursor() {
        let next = AtomicU64::new(END - 4096);
        for length in [0, 1, 8192, u64::MAX] {
            assert_eq!(allocate(&next, length), None);
            assert_eq!(next.load(Ordering::Relaxed), END - 4096);
        }
        assert_eq!(allocate(&next, 4096), Some(END - 4096));
        assert_eq!(allocate(&next, 4096), None);
    }
}
