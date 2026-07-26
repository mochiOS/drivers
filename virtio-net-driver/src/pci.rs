use mochi_user_platform as platform;
use plugkit::PciConfig;
use plugkit::virtio::{
    CapabilityRegion, PciAddress, PciBar, PciConfigIo, PciTransportAccess, VirtioError,
    VirtioPciCapabilities, VirtioResult, find_pci_device,
};

const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
const PCI_CONFIG_DATA: u16 = 0x0cfc;
pub(crate) const VIRTIO_VENDOR_ID: u16 = 0x1af4;
pub(crate) const VIRTIO_NET_DEVICE_ID: u16 = 0x1041;
const MMIO_VIRTUAL_BASE: u64 = 0x0000_6200_0000_0000;
const MMIO_BAR_SPACING: u64 = 0x0200_0000;
const MAX_MAPPED_BAR_SIZE: u64 = 0x0100_0000;

pub(crate) struct PciPorts;
impl PciPorts {
    fn address(a: PciAddress, o: u16) -> VirtioResult<u32> {
        if o >= 256 || o & 3 != 0 {
            return Err(VirtioError::AccessFailed);
        }
        Ok(0x8000_0000
            | (u32::from(a.bus) << 16)
            | (u32::from(a.device) << 11)
            | (u32::from(a.function) << 8)
            | u32::from(o))
    }
    fn read(port: u16) -> VirtioResult<u32> {
        platform::syscall::call2(platform::syscall::SyscallNumber::PortIn, u64::from(port), 4)
            .map(|v| v as u32)
            .map_err(|_| VirtioError::AccessFailed)
    }
    fn write(port: u16, v: u32) -> VirtioResult<()> {
        platform::syscall::call3(
            platform::syscall::SyscallNumber::PortOut,
            u64::from(port),
            u64::from(v),
            4,
        )
        .map(|_| ())
        .map_err(|_| VirtioError::AccessFailed)
    }
}
impl PciConfigIo for PciPorts {
    fn read_u32(&mut self, a: PciAddress, o: u16) -> VirtioResult<u32> {
        Self::write(PCI_CONFIG_ADDRESS, Self::address(a, o)?)?;
        Self::read(PCI_CONFIG_DATA)
    }
    fn write_u32(&mut self, a: PciAddress, o: u16, v: u32) -> VirtioResult<()> {
        Self::write(PCI_CONFIG_ADDRESS, Self::address(a, o)?)?;
        Self::write(PCI_CONFIG_DATA, v)
    }
}

#[derive(Clone, Copy)]
struct Mapping {
    virtual_start: u64,
    register_base: u64,
    mapped_size: u64,
    register_size: u64,
}
pub(crate) struct MappedBars {
    mappings: [Option<Mapping>; 6],
}
impl MappedBars {
    fn map(bars: &[PciBar], caps: VirtioPciCapabilities) -> VirtioResult<Self> {
        let mut mappings = [None; 6];
        for region in required(caps).into_iter().flatten() {
            let i = usize::from(region.bar);
            if i >= mappings.len() {
                return Err(VirtioError::InvalidBar);
            }
            if mappings[i].is_some() {
                continue;
            }
            let bar = bars
                .iter()
                .find(|b| b.index == region.bar)
                .ok_or(VirtioError::InvalidBar)?;
            if bar.is_io || bar.size == 0 || bar.size > MAX_MAPPED_BAR_SIZE {
                return Err(VirtioError::InvalidBar);
            }
            let physical = bar.address & !0xfff;
            let page_offset = bar.address - physical;
            let mapped = align(
                bar.size
                    .checked_add(page_offset)
                    .ok_or(VirtioError::RegionOverflow)?,
            )?;
            let virtual_start = MMIO_VIRTUAL_BASE
                .checked_add(u64::from(region.bar) * MMIO_BAR_SPACING)
                .ok_or(VirtioError::RegionOverflow)?;
            platform::memory::map_physical_range(virtual_start, physical, mapped)
                .map_err(|_| VirtioError::AccessFailed)?;
            mappings[i] = Some(Mapping {
                virtual_start,
                register_base: virtual_start + page_offset,
                mapped_size: mapped,
                register_size: bar.size,
            })
        }
        Ok(Self { mappings })
    }
    fn pointer(&self, bar: u8, offset: u32, size: u64) -> VirtioResult<*mut u8> {
        let m = self
            .mappings
            .get(usize::from(bar))
            .and_then(|m| *m)
            .ok_or(VirtioError::InvalidBar)?;
        let end = u64::from(offset)
            .checked_add(size)
            .ok_or(VirtioError::RegionOverflow)?;
        if end > m.register_size {
            return Err(VirtioError::RegisterOutOfBounds);
        }
        Ok((m.register_base + u64::from(offset)) as *mut u8)
    }
}
impl PciTransportAccess for MappedBars {
    fn read_u8(&mut self, b: u8, o: u32) -> VirtioResult<u8> {
        Ok(unsafe { core::ptr::read_volatile(self.pointer(b, o, 1)?) })
    }
    fn read_u16(&mut self, b: u8, o: u32) -> VirtioResult<u16> {
        Ok(u16::from_le(unsafe {
            core::ptr::read_volatile(self.pointer(b, o, 2)?.cast())
        }))
    }
    fn read_u32(&mut self, b: u8, o: u32) -> VirtioResult<u32> {
        Ok(u32::from_le(unsafe {
            core::ptr::read_volatile(self.pointer(b, o, 4)?.cast())
        }))
    }
    fn write_u8(&mut self, b: u8, o: u32, v: u8) -> VirtioResult<()> {
        unsafe { core::ptr::write_volatile(self.pointer(b, o, 1)?, v) };
        Ok(())
    }
    fn write_u16(&mut self, b: u8, o: u32, v: u16) -> VirtioResult<()> {
        unsafe { core::ptr::write_volatile(self.pointer(b, o, 2)?.cast(), v.to_le()) };
        Ok(())
    }
    fn write_u32(&mut self, b: u8, o: u32, v: u32) -> VirtioResult<()> {
        unsafe { core::ptr::write_volatile(self.pointer(b, o, 4)?.cast(), v.to_le()) };
        Ok(())
    }
}
impl Drop for MappedBars {
    fn drop(&mut self) {
        for m in self.mappings.into_iter().flatten() {
            let _ = platform::memory::munmap(m.virtual_start, m.mapped_size);
        }
    }
}
pub(crate) fn connect() -> VirtioResult<(VirtioPciCapabilities, MappedBars, PciAddress)> {
    let mut ports = PciPorts;
    let dev = find_pci_device(&mut ports, VIRTIO_VENDOR_ID, VIRTIO_NET_DEVICE_ID)?
        .ok_or(VirtioError::QueueUnavailable)?;
    let address = dev.address;
    let bars = dev.probe_bars(&mut ports)?;
    let config: PciConfig = dev.read_config(&mut ports)?;
    let caps = VirtioPciCapabilities::parse(&config, &bars)?;
    let mapped = MappedBars::map(&bars, caps)?;
    dev.enable_memory_and_bus_master(&mut ports)?;
    Ok((caps, mapped, address))
}
fn required(c: VirtioPciCapabilities) -> [Option<CapabilityRegion>; 4] {
    [Some(c.common), Some(c.notify), Some(c.isr), c.device]
}
fn align(v: u64) -> VirtioResult<u64> {
    v.checked_add(0xfff)
        .map(|x| x & !0xfff)
        .ok_or(VirtioError::RegionOverflow)
}
