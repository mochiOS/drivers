use crate::dma::DmaRegion;
use crate::pci::{self, MappedBars};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use mochios_net_device_protocol::{DeviceStatistics, InterfaceInfo, MAX_FRAME_LEN};
use plugkit::prelude::*;
use plugkit::virtio::{
    Descriptor, DmaMemory, FeatureSet, PciTransportAccess, SplitVirtqueue, VirtioDevice,
    VirtioError, VirtioPciTransport, VirtqueueLayout,
};
use virtio_net_driver::{
    REQUESTED_FEATURES, REQUIRED_FEATURES, VIRTIO_NET_HEADER_LEN, received_frame_range,
};

const VIRTIO_NET_F_STATUS: u64 = virtio_net_driver::VIRTIO_NET_F_STATUS;
const RX_QUEUE: u16 = 0;
const TX_QUEUE: u16 = 1;
const MAX_QUEUE_SIZE: u16 = 64;
const RX_BUFFER_COUNT: usize = 32;
const RX_QUEUE_LIMIT: usize = 64;
const BUFFER_LEN: usize = VIRTIO_NET_HEADER_LEN + MAX_FRAME_LEN;

#[derive(Debug)]
pub(crate) enum NetError {
    Virtio(VirtioError),
    System(u64),
    InvalidConfig,
    QueueExhausted,
    InvalidFrame,
}
impl From<VirtioError> for NetError {
    fn from(v: VirtioError) -> Self {
        Self::Virtio(v)
    }
}

impl NetError {
    pub(crate) fn errno(&self) -> u64 {
        match self {
            Self::Virtio(error) => {
                let _ = error;
                mochi_user_syscall::EIO
            }
            Self::System(errno) => *errno,
            Self::InvalidConfig | Self::InvalidFrame => mochi_user_syscall::EINVAL,
            Self::QueueExhausted => mochi_user_syscall::EAGAIN,
        }
    }
}

struct BufferSlot {
    dma: DmaRegion,
    head: Option<u16>,
}
pub(crate) struct NetDevice {
    device: VirtioDevice<MappedBars>,
    rx: SplitVirtqueue<DmaRegion>,
    tx: SplitVirtqueue<DmaRegion>,
    rx_notify: u16,
    tx_notify: u16,
    rx_buffers: Vec<BufferSlot>,
    tx_buffers: Vec<BufferSlot>,
    received: VecDeque<Vec<u8>>,
    info: InterfaceInfo,
    stats: DeviceStatistics,
}

impl NetDevice {
    pub(crate) fn initialize() -> Result<Self, NetError> {
        let (caps, bars, address) = pci::connect()?;
        let device_cfg = caps.device.ok_or(NetError::InvalidConfig)?;
        let mut device = VirtioDevice::new(VirtioPciTransport::new(caps, bars));
        device.begin_initialization()?;
        let requested = FeatureSet::new(REQUESTED_FEATURES);
        let required = FeatureSet::new(REQUIRED_FEATURES);
        let negotiated = device.negotiate_features(requested, required)?;
        let mut mac = [0; 6];
        for (i, byte) in mac.iter_mut().enumerate() {
            *byte = device
                .transport_mut()
                .access_mut()
                .read_u8(device_cfg.bar, device_cfg.offset + i as u32)?;
        }
        let link_up = if negotiated.contains_all(FeatureSet::new(VIRTIO_NET_F_STATUS)) {
            device
                .transport_mut()
                .access_mut()
                .read_u16(device_cfg.bar, device_cfg.offset + 6)?
                & 1
                != 0
        } else {
            true
        };
        let (rx, rx_notify) = make_queue(&mut device, RX_QUEUE)?;
        let (tx, tx_notify) = make_queue(&mut device, TX_QUEUE)?;
        let rx_buffer_count = RX_BUFFER_COUNT.min(usize::from(rx.size()));
        let tx_buffer_count = RX_BUFFER_COUNT.min(usize::from(tx.size()));
        device.finish_initialization()?;
        let mut this = Self {
            device,
            rx,
            tx,
            rx_notify,
            tx_notify,
            rx_buffers: Vec::with_capacity(rx_buffer_count),
            tx_buffers: Vec::with_capacity(tx_buffer_count),
            received: VecDeque::with_capacity(RX_QUEUE_LIMIT),
            info: InterfaceInfo {
                interface_id: 1,
                mac,
                link_up,
                mtu: 1500,
                driver_id: 0x766e6574,
                device_id: (u32::from(address.bus) << 16)
                    | (u32::from(address.device) << 8)
                    | u32::from(address.function),
                driver_name: *b"virtio-net\0\0",
            },
            stats: DeviceStatistics::default(),
        };
        for _ in 0..rx_buffer_count {
            this.rx_buffers.push(BufferSlot {
                dma: DmaRegion::allocate(BUFFER_LEN).map_err(NetError::System)?,
                head: None,
            });
        }
        for _ in 0..tx_buffer_count {
            this.tx_buffers.push(BufferSlot {
                dma: DmaRegion::allocate(BUFFER_LEN).map_err(NetError::System)?,
                head: None,
            });
        }
        for slot in 0..rx_buffer_count {
            this.post_rx(slot)?;
        }
        this.device
            .transport_mut()
            .notify_queue(RX_QUEUE, this.rx_notify)?;
        let mut spec = DeviceSpec::new(
            "/pci/virtio-net0",
            "virtio-net",
            DeviceBus::Pci,
            DeviceClass::Network,
        );
        spec.vendor_id = Some(u32::from(pci::VIRTIO_VENDOR_ID));
        spec.device_id = Some(u32::from(pci::VIRTIO_NET_DEVICE_ID));
        spec.properties.insert(
            "network.mac".into(),
            DeviceProperty::Bytes(DeviceBytes::new(mac.to_vec())),
        );
        let _ = register_device(spec);
        Ok(this)
    }
    pub(crate) const fn info(&self) -> InterfaceInfo {
        self.info
    }
    pub(crate) const fn statistics(&self) -> DeviceStatistics {
        self.stats
    }
    fn post_rx(&mut self, slot: usize) -> Result<(), NetError> {
        let b = self
            .rx_buffers
            .get_mut(slot)
            .ok_or(NetError::InvalidFrame)?;
        b.dma.bytes_mut().fill(0);
        b.dma.sync_for_device()?;
        let head = self.rx.enqueue(&[Descriptor {
            address: b.dma.device_address(),
            length: BUFFER_LEN as u32,
            device_writable: true,
        }])?;
        b.head = Some(head);
        Ok(())
    }
    pub(crate) fn poll(&mut self) -> Result<(), NetError> {
        while let Some(used) = self.rx.pop_used()? {
            let Some(slot) = self
                .rx_buffers
                .iter()
                .position(|b| b.head == Some(used.head))
            else {
                self.stats.rx_errors += 1;
                return Err(NetError::InvalidFrame);
            };
            self.rx_buffers[slot].head = None;
            self.rx_buffers[slot].dma.sync_for_cpu()?;
            let written = used.written as usize;
            let frame_range = received_frame_range(written, BUFFER_LEN);
            if frame_range.is_err() {
                self.stats.rx_errors += 1;
            } else if self.received.len() >= RX_QUEUE_LIMIT {
                self.stats.rx_dropped += 1;
            } else {
                let range = frame_range.map_err(|_| NetError::InvalidFrame)?;
                let frame = self.rx_buffers[slot].dma.bytes()[range].to_vec();
                self.stats.rx_packets += 1;
                self.stats.rx_bytes += frame.len() as u64;
                self.received.push_back(frame)
            }
            self.post_rx(slot)?;
            self.device
                .transport_mut()
                .notify_queue(RX_QUEUE, self.rx_notify)?;
        }
        while let Some(used) = self.tx.pop_used()? {
            let Some(slot) = self
                .tx_buffers
                .iter()
                .position(|b| b.head == Some(used.head))
            else {
                self.stats.tx_errors += 1;
                return Err(NetError::InvalidFrame);
            };
            self.tx_buffers[slot].head = None;
        }
        Ok(())
    }
    pub(crate) fn receive(&mut self) -> Option<Vec<u8>> {
        self.received.pop_front()
    }
    pub(crate) fn transmit(&mut self, frame: &[u8]) -> Result<(), NetError> {
        if frame.len() < 14 || frame.len() > MAX_FRAME_LEN {
            self.stats.tx_errors += 1;
            return Err(NetError::InvalidFrame);
        }
        self.poll()?;
        let Some(slot) = self.tx_buffers.iter().position(|b| b.head.is_none()) else {
            self.stats.tx_dropped += 1;
            return Err(NetError::QueueExhausted);
        };
        let b = &mut self.tx_buffers[slot];
        b.dma.bytes_mut()[..VIRTIO_NET_HEADER_LEN].fill(0);
        b.dma.bytes_mut()[VIRTIO_NET_HEADER_LEN..VIRTIO_NET_HEADER_LEN + frame.len()]
            .copy_from_slice(frame);
        b.dma.sync_for_device()?;
        let head = self.tx.enqueue(&[Descriptor {
            address: b.dma.device_address(),
            length: (VIRTIO_NET_HEADER_LEN + frame.len()) as u32,
            device_writable: false,
        }])?;
        b.head = Some(head);
        self.device
            .transport_mut()
            .notify_queue(TX_QUEUE, self.tx_notify)?;
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += frame.len() as u64;
        Ok(())
    }
    pub(crate) fn fail(&mut self) {
        self.device.fail()
    }
}
fn make_queue(
    device: &mut VirtioDevice<MappedBars>,
    index: u16,
) -> Result<(SplitVirtqueue<DmaRegion>, u16), NetError> {
    let maximum = device.transport_mut().queue_max_size(index)?;
    let size = maximum.min(MAX_QUEUE_SIZE);
    if size < 2 {
        return Err(NetError::Virtio(VirtioError::InvalidQueueSize));
    }
    let size = 1u16 << (15 - size.leading_zeros() as u16);
    let layout = VirtqueueLayout::calculate(size)?;
    let memory = DmaRegion::allocate(layout.total_size).map_err(NetError::System)?;
    let queue = SplitVirtqueue::new(memory, size)?;
    let notify = device.transport_mut().configure_queue(
        index,
        size,
        queue.descriptor_address()?,
        queue.available_address()?,
        queue.used_address()?,
    )?;
    Ok((queue, notify))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_supported_features_are_selected() {
        let offered = REQUESTED_FEATURES | 1 | (1 << 15);
        assert_eq!(
            virtio_net_driver::selected_features(offered),
            Some(REQUESTED_FEATURES)
        )
    }
    #[test]
    fn buffers_are_bounded() {
        assert!(RX_BUFFER_COUNT < RX_QUEUE_LIMIT);
        assert_eq!(BUFFER_LEN, 1526)
    }
}
