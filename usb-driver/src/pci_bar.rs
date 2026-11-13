//! Decode the PCI owner's immutable BAR sizing masks, without a guessed extent.

pub(crate) fn memory_extent(
    low: u32,
    high: u32,
    mask_low: u32,
    mask_high: u32,
) -> Option<(u64, u64)> {
    if low & 1 != 0 || !matches!((low >> 1) & 3, 0 | 2) {
        return None;
    }
    let wide = (low >> 1) & 3 == 2;
    let base = u64::from(low & !15) | if wide { u64::from(high) << 32 } else { 0 };
    let size = if wide {
        let mask = u64::from(mask_low & !15) | (u64::from(mask_high) << 32);
        mask.wrapping_neg()
    } else {
        u64::from((mask_low & !15).wrapping_neg())
    };
    if base == 0 || !size.is_power_of_two() || base & (size - 1) != 0 {
        return None;
    }
    Some((base, size))
}

#[cfg(test)]
mod tests {
    use super::memory_extent;

    #[test]
    fn qemu_xhci_is_sixteen_kib_not_the_previous_sixty_four_kib_guess() {
        assert_eq!(
            memory_extent(0xc004, 0x70, 0xffff_c004, u32::MAX),
            Some((0x70_0000_c000, 16 * 1024))
        );
    }

    #[test]
    fn supports_both_pci_widths_and_large_extents() {
        assert_eq!(
            memory_extent(0xc000_0000, 0, 0xfff0_0000, 0),
            Some((0xc000_0000, 1 << 20))
        );
        assert_eq!(
            memory_extent(4, 8, 4, 0xffff_fffe),
            Some((8 << 32, 1 << 33))
        );
    }

    #[test]
    fn rejects_io_reserved_unassigned_misaligned_and_invalid_masks() {
        for values in [
            (1, 0, u32::MAX, 0),
            (6, 0, u32::MAX, 0),
            (0, 0, 0xffff_c000, 0),
            (0xc000_1000, 0, 0xffff_c000, 0),
            (4, 0x70, 0, 0),
            (4, 0x70, 0xffff_8004, 0xffff_fffe),
        ] {
            assert_eq!(memory_extent(values.0, values.1, values.2, values.3), None);
        }
    }
}
