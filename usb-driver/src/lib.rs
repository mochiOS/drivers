#![no_std]

extern crate alloc;

use alloc::format;
use core::mem::size_of;
use core::ptr::{read_volatile, write_volatile};
use mochi_user_platform as platform;
use mochi_user_syscall as syscall;
use plugkit::prelude::*;

const PCI_CFG_ADDR: u16 = 0xCF8;
const PCI_CFG_DATA: u16 = 0xCFC;
const XHCI_PROG_IF: u8 = 0x30;

const PROT_READ: u64 = 0x1;
const PROT_WRITE: u64 = 0x2;
const MAP_PRIVATE: u64 = 0x2;
const MAP_ANONYMOUS: u64 = 0x20;

const XHCI_CAP_CAPLENGTH: usize = 0x00;
const XHCI_CAP_HCIVERSION: usize = 0x02;
const XHCI_CAP_HCSPARAMS1: usize = 0x04;
const XHCI_CAP_HCCPARAMS1: usize = 0x10;
const XHCI_CAP_DBOFF: usize = 0x14;
const XHCI_CAP_RTSOFF: usize = 0x18;

const XHCI_OP_USBCMD: usize = 0x00;
const XHCI_OP_USBSTS: usize = 0x04;
const XHCI_OP_PAGESIZE: usize = 0x08;
const XHCI_OP_DNCTRL: usize = 0x14;
const XHCI_OP_CRCR: usize = 0x18;
const XHCI_OP_DCBAAP: usize = 0x30;
const XHCI_OP_CONFIG: usize = 0x38;
const XHCI_OP_PORTSC_BASE: usize = 0x400;
const XHCI_OP_PORTSC_STRIDE: usize = 0x10;
const XHCI_USBCMD_RUN: u32 = 1 << 0;
const XHCI_USBCMD_HCRST: u32 = 1 << 1;
const XHCI_USBSTS_HCHALTED: u32 = 1 << 0;
const XHCI_USBSTS_CNR: u32 = 1 << 11;

const XHCI_RT_IR0: usize = 0x20;
const XHCI_IR_IMAN: usize = 0x00;
const XHCI_IR_IMOD: usize = 0x04;
const XHCI_IR_ERSTSZ: usize = 0x08;
const XHCI_IR_ERSTBA: usize = 0x10;
const XHCI_IR_ERDP: usize = 0x18;

const XHCI_TRB_TYPE_LINK: u32 = 6;
const XHCI_TRB_TYPE_NORMAL: u32 = 1;
const XHCI_TRB_TYPE_ENABLE_SLOT: u32 = 9;
const XHCI_TRB_TYPE_ADDRESS_DEVICE: u32 = 11;
const XHCI_TRB_TYPE_CONFIGURE_ENDPOINT: u32 = 12;
const XHCI_TRB_TYPE_SETUP_STAGE: u32 = 2;
const XHCI_TRB_TYPE_DATA_STAGE: u32 = 3;
const XHCI_TRB_TYPE_STATUS_STAGE: u32 = 4;
const XHCI_TRB_TYPE_TRANSFER_EVENT: u32 = 32;
const XHCI_TRB_TYPE_COMMAND_COMPLETION: u32 = 33;
const XHCI_TRB_CYCLE: u32 = 1 << 0;
const XHCI_TRB_TC: u32 = 1 << 1;
const XHCI_TRB_ISP: u32 = 1 << 2;
const XHCI_TRB_IOC: u32 = 1 << 5;
const XHCI_TRB_IDT: u32 = 1 << 6;
const XHCI_TRB_DIR_IN: u32 = 1 << 16;
const XHCI_CC_SUCCESS: u32 = 1;

const XHCI_CTX_SIZE_64: u32 = 1 << 2;
const XHCI_INPUT_CONTROL_DROP_FLAGS: usize = 0x00;
const XHCI_INPUT_CONTROL_ADD_FLAGS: usize = 0x04;
const XHCI_INPUT_CONTROL_CONFIG_VALUE: usize = 0x1c;
const XHCI_SLOT_CTX_DW0: usize = 0x00;
const XHCI_SLOT_CTX_DW1: usize = 0x04;
const XHCI_EP_CTX_DW0: usize = 0x00;
const XHCI_EP_CTX_DW1: usize = 0x04;
const XHCI_EP_CTX_TR_DEQUEUE_LO: usize = 0x08;
const XHCI_EP_CTX_TR_DEQUEUE_HI: usize = 0x0c;
const XHCI_EP_CTX_DW4: usize = 0x10;
const XHCI_EP_TYPE_CONTROL_BIDIR: u32 = 4;
const XHCI_EP_TYPE_INTERRUPT_IN: u32 = 7;
const XHCI_PORTSC_CCS: u32 = 1 << 0;
const XHCI_PORTSC_PED: u32 = 1 << 1;
const XHCI_PORTSC_OCA: u32 = 1 << 3;
const XHCI_PORTSC_PR: u32 = 1 << 4;
const XHCI_PORTSC_PP: u32 = 1 << 9;
const XHCI_PORTSC_SPEED_SHIFT: u32 = 10;
const XHCI_PORTSC_SPEED_MASK: u32 = 0xF << XHCI_PORTSC_SPEED_SHIFT;
const XHCI_PORTSC_CHANGE_BITS: u32 =
    (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20) | (1 << 21) | (1 << 22) | (1 << 23);
const USB_DESC_TYPE_DEVICE: u16 = 1;
const USB_DESC_TYPE_CONFIGURATION: u16 = 2;
const USB_DESC_TYPE_INTERFACE: u8 = 4;
const USB_DESC_TYPE_ENDPOINT: u8 = 5;
const USB_DESC_TYPE_HID: u8 = 0x21;
const USB_DESC_TYPE_REPORT: u16 = 0x22;
const HID_REQ_SET_IDLE: u8 = 0x0A;
const HID_REQ_SET_PROTOCOL: u8 = 0x0B;

#[derive(Clone, Copy)]
struct InterruptEndpointInfo {
    address: u8,
    max_packet: u16,
    interval: u8,
}

struct ConfigureEndpointCtx<'a> {
    mmio: &'a MmioRegion,
    rtsoff: usize,
    command_ring: &'a DmaPage,
    ring_state: &'a mut CommandRingState,
    event_ring: &'a DmaPage,
    output_ctx: &'a DmaPage,
    input_ctx: &'a DmaPage,
    slot_id: u8,
    hccparams1: u32,
    root_port: u8,
    speed_id: u32,
}

#[derive(Clone, Copy)]
struct UsbConfigurationInfo {
    config_value: u8,
    interface_number: u8,
    interface_class: u8,
    interface_subclass: u8,
    interface_protocol: u8,
    report_descriptor_len: u16,
    interrupt_in: Option<InterruptEndpointInfo>,
}

#[derive(Clone, Copy)]
struct PciLocation {
    bus: u8,
    device: u8,
    function: u8,
}

#[derive(Clone, Copy)]
struct XhciBar {
    phys_base: u64,
    size: u64,
}

struct MmioRegion {
    virt_base: usize,
    len: usize,
}

impl MmioRegion {
    fn map(phys_base: u64, len: u64) -> Result<Self, syscall::SysError> {
        let page_base = phys_base & !0xfff;
        let page_offset = (phys_base & 0xfff) as usize;
        let span = page_offset
            .checked_add(len as usize)
            .ok_or_else(|| syscall::SysError::from_raw(syscall::EINVAL as i64))?;
        let map_len = ((span + 0xfff) & !0xfff) as u64;
        let virt_base = platform::memory::mmap(
            0,
            map_len,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            0,
        )?;
        platform::memory::map_physical_range(virt_base, page_base, map_len)?;
        Ok(Self {
            virt_base: virt_base as usize + page_offset,
            len: len as usize,
        })
    }

    fn subregion(&self, offset: usize, len: usize) -> Option<Self> {
        let end = offset.checked_add(len)?;
        if end > self.len {
            return None;
        }
        Some(Self {
            virt_base: self.virt_base + offset,
            len,
        })
    }

    fn read_u8(&self, offset: usize) -> u8 {
        debug_assert!(offset < self.len);
        // SAFETY: MMIO region was explicitly mapped read/write for this process.
        unsafe { read_volatile((self.virt_base + offset) as *const u8) }
    }

    fn read_u16(&self, offset: usize) -> u16 {
        debug_assert!(offset + 2 <= self.len);
        // SAFETY: MMIO region was explicitly mapped read/write for this process.
        unsafe { read_volatile((self.virt_base + offset) as *const u16) }
    }

    fn read_u32(&self, offset: usize) -> u32 {
        debug_assert!(offset + 4 <= self.len);
        // SAFETY: MMIO region was explicitly mapped read/write for this process.
        unsafe { read_volatile((self.virt_base + offset) as *const u32) }
    }

    fn read_u64(&self, offset: usize) -> u64 {
        debug_assert!(offset + 8 <= self.len);
        // SAFETY: MMIO region was explicitly mapped read/write for this process.
        unsafe { read_volatile((self.virt_base + offset) as *const u64) }
    }

    #[allow(dead_code)]
    fn write_u32(&self, offset: usize, value: u32) {
        debug_assert!(offset + 4 <= self.len);
        // SAFETY: MMIO region was explicitly mapped read/write for this process.
        unsafe { write_volatile((self.virt_base + offset) as *mut u32, value) }
    }

    fn write_u64(&self, offset: usize, value: u64) {
        debug_assert!(offset + 8 <= self.len);
        // SAFETY: MMIO region was explicitly mapped read/write for this process.
        unsafe { write_volatile((self.virt_base + offset) as *mut u64, value) }
    }
}

struct DmaPage {
    handle: u64,
    virt: u64,
    phys: u64,
    len: usize,
}

struct CommandRingState {
    next_index: usize,
    cycle: u32,
}

static mut XHCI_COMMAND_RING_STATE: CommandRingState = CommandRingState {
    next_index: 0,
    cycle: XHCI_TRB_CYCLE,
};
static mut XHCI_DOORBELL_BASE: usize = 0;
static mut PCI_CONFIG_IO_FAILED_LOGGED: bool = false;

impl DmaPage {
    fn allocate(len: usize) -> Result<Self, syscall::SysError> {
        let alloc = platform::memory::dma_alloc(len as u64)?;
        Ok(Self {
            handle: alloc.handle,
            virt: alloc.virt_addr,
            phys: alloc.phys_addr,
            len: alloc.len as usize,
        })
    }

    fn ptr(&self) -> *mut u8 {
        self.virt as *mut u8
    }

    fn write_u32(&self, offset: usize, value: u32) {
        debug_assert!(offset + 4 <= self.len);
        // SAFETY: DMA page is mapped writable in this process.
        unsafe { write_volatile(self.ptr().add(offset) as *mut u32, value) }
    }

    fn write_u64(&self, offset: usize, value: u64) {
        debug_assert!(offset + 8 <= self.len);
        // SAFETY: DMA page is mapped writable in this process.
        unsafe { write_volatile(self.ptr().add(offset) as *mut u64, value) }
    }

    fn read_u32(&self, offset: usize) -> u32 {
        debug_assert!(offset + 4 <= self.len);
        // SAFETY: DMA page is mapped readable in this process.
        unsafe { read_volatile(self.ptr().add(offset) as *const u32) }
    }

    fn read_u8(&self, offset: usize) -> u8 {
        debug_assert!(offset < self.len);
        // SAFETY: DMA page is mapped readable in this process.
        unsafe { read_volatile(self.ptr().add(offset) as *const u8) }
    }

    fn zero(&self) {
        // SAFETY: DMA page is mapped writable in this process.
        unsafe { core::ptr::write_bytes(self.ptr(), 0, self.len) }
    }

    fn copy_from(&self, other: &DmaPage) {
        debug_assert!(self.len == other.len);
        for offset in 0..self.len {
            // SAFETY: both DMA pages are mapped into this process and offset is bounds-checked.
            let byte = unsafe { read_volatile(other.ptr().add(offset) as *const u8) };
            // SAFETY: both DMA pages are mapped into this process and offset is bounds-checked.
            unsafe { write_volatile(self.ptr().add(offset), byte) };
        }
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
struct Trb {
    parameter: u64,
    status: u32,
    control: u32,
}

struct TransferRingState {
    next_index: usize,
    cycle: u32,
}

fn wait_until(label: &str, limit: usize, mut pred: impl FnMut() -> bool) -> bool {
    for _ in 0..limit {
        if pred() {
            return true;
        }
        platform::thread::yield_now();
    }
    let _ = label;
    false
}

fn xhci_stop_and_reset(mmio: &MmioRegion, cap_length: usize) -> bool {
    let mut usbcmd = mmio.read_u32(cap_length + XHCI_OP_USBCMD);
    if (usbcmd & XHCI_USBCMD_RUN) != 0 {
        usbcmd &= !XHCI_USBCMD_RUN;
        mmio.write_u32(cap_length + XHCI_OP_USBCMD, usbcmd);
    }
    if !wait_until("xhci halt", 100_000, || {
        (mmio.read_u32(cap_length + XHCI_OP_USBSTS) & XHCI_USBSTS_HCHALTED) != 0
    }) {
        return false;
    }

    mmio.write_u32(
        cap_length + XHCI_OP_USBCMD,
        mmio.read_u32(cap_length + XHCI_OP_USBCMD) | XHCI_USBCMD_HCRST,
    );
    if !wait_until("xhci reset clear", 100_000, || {
        (mmio.read_u32(cap_length + XHCI_OP_USBCMD) & XHCI_USBCMD_HCRST) == 0
    }) {
        return false;
    }
    wait_until("xhci controller ready", 100_000, || {
        (mmio.read_u32(cap_length + XHCI_OP_USBSTS) & XHCI_USBSTS_CNR) == 0
    })
}

fn xhci_start(mmio: &MmioRegion, cap_length: usize) -> bool {
    mmio.write_u32(
        cap_length + XHCI_OP_USBCMD,
        mmio.read_u32(cap_length + XHCI_OP_USBCMD) | XHCI_USBCMD_RUN,
    );
    wait_until("xhci run", 100_000, || {
        (mmio.read_u32(cap_length + XHCI_OP_USBSTS) & XHCI_USBSTS_HCHALTED) == 0
    })
}

#[inline(never)]
unsafe extern "C" fn write_doorbell(base: usize, index: u32, value: u32) {
    // SAFETY: caller provides a mapped xHCI doorbell base and valid doorbell index.
    unsafe { write_volatile((base + (index as usize) * 4) as *mut u32, value) }
}

fn current_doorbell_base() -> usize {
    // SAFETY: initialized once after MMIO map, then read-only.
    unsafe { XHCI_DOORBELL_BASE }
}

fn doorbell_value(target: u32, stream_id: u16) -> u32 {
    target | ((stream_id as u32) << 16)
}

fn endpoint_dci(address: u8) -> u32 {
    let endpoint_number = u32::from(address & 0x0f);
    let direction_in = (address & 0x80) != 0;
    endpoint_number * 2 + if direction_in { 1 } else { 0 }
}

fn queue_command_trb(
    command_ring: &DmaPage,
    state: &mut CommandRingState,
    parameter: u64,
    status: u32,
    control: u32,
) {
    let trb_count = command_ring.len / size_of::<Trb>();
    debug_assert!(state.next_index < trb_count - 1);
    let offset = state.next_index * size_of::<Trb>();
    command_ring.write_u64(offset, parameter);
    command_ring.write_u32(offset + 8, status);
    command_ring.write_u32(offset + 12, control | state.cycle);
    state.next_index += 1;
}

fn queue_transfer_trb(
    transfer_ring: &DmaPage,
    state: &mut TransferRingState,
    parameter: u64,
    status: u32,
    control: u32,
) {
    let trb_count = transfer_ring.len / size_of::<Trb>();
    debug_assert!(state.next_index < trb_count - 1);
    let offset = state.next_index * size_of::<Trb>();
    transfer_ring.write_u64(offset, parameter);
    transfer_ring.write_u32(offset + 8, status);
    transfer_ring.write_u32(offset + 12, control | state.cycle);
    state.next_index += 1;
}

fn clear_event_trbs(event_ring: &DmaPage) {
    let trb_size = size_of::<Trb>();
    let event_count = event_ring.len / trb_size;
    for idx in 0..event_count {
        event_ring.write_u32(idx * trb_size + 12, 0);
    }
}

fn wait_command_completion(
    mmio: &MmioRegion,
    rtsoff: usize,
    event_ring: &DmaPage,
    expected_type: u32,
) -> Option<(u32, u8)> {
    let trb_size = size_of::<Trb>();
    let event_count = event_ring.len / trb_size;
    wait_until("command completion", 100_000, || {
        for idx in 0..event_count {
            let control = event_ring.read_u32(idx * trb_size + 12);
            if (control & XHCI_TRB_CYCLE) != 0 {
                return true;
            }
        }
        false
    });

    let mut completion = None;
    let mut consumed = 0usize;
    for idx in 0..event_count {
        let offset = idx * trb_size;
        let control = event_ring.read_u32(offset + 12);
        if (control & XHCI_TRB_CYCLE) == 0 {
            continue;
        }
        consumed = idx + 1;
        let trb_type = (control >> 10) & 0x3f;
        if trb_type == expected_type {
            let status = event_ring.read_u32(offset + 8);
            let completion_code = (status >> 24) & 0xff;
            let slot_id = ((control >> 24) & 0xff) as u8;
            completion = Some((completion_code, slot_id));
        }
        event_ring.write_u32(offset + 12, 0);
    }

    if consumed != 0 {
        let next_offset = if consumed >= event_count { 0 } else { consumed * trb_size };
        let ir0 = rtsoff + XHCI_RT_IR0;
        mmio.write_u64(ir0 + XHCI_IR_ERDP, event_ring.phys + next_offset as u64);
    }

    completion
}

fn wait_transfer_event(
    mmio: &MmioRegion,
    rtsoff: usize,
    event_ring: &DmaPage,
    slot_id: u8,
) -> Option<u32> {
    let trb_size = size_of::<Trb>();
    let event_count = event_ring.len / trb_size;
    let ready = wait_until("transfer event", 500, || {
        for idx in 0..event_count {
            let control = event_ring.read_u32(idx * trb_size + 12);
            if (control & XHCI_TRB_CYCLE) != 0 {
                return true;
            }
        }
        false
    });
    if !ready {
        let _ = slot_id;
        return None;
    }
    let mut completion = None;
    let mut consumed = 0usize;
    for idx in 0..event_count {
        let offset = idx * trb_size;
        let control = event_ring.read_u32(offset + 12);
        if (control & XHCI_TRB_CYCLE) == 0 {
            continue;
        }
        consumed = idx + 1;
        let trb_type = (control >> 10) & 0x3f;
        let event_slot_id = ((control >> 24) & 0xff) as u8;
        if trb_type == XHCI_TRB_TYPE_TRANSFER_EVENT && event_slot_id == slot_id {
            let status = event_ring.read_u32(offset + 8);
            completion = Some((status >> 24) & 0xff);
        }
        event_ring.write_u32(offset + 12, 0);
    }

    if consumed != 0 {
        let next_offset = if consumed >= event_count { 0 } else { consumed * trb_size };
        let ir0 = rtsoff + XHCI_RT_IR0;
        mmio.write_u64(ir0 + XHCI_IR_ERDP, event_ring.phys + next_offset as u64);
    }

    completion
}

fn enable_slot(
    mmio: &MmioRegion,
    dboff: usize,
    rtsoff: usize,
    command_ring: &DmaPage,
    ring_state: &mut CommandRingState,
    event_ring: &DmaPage,
) -> Option<u8> {
    queue_command_trb(
        command_ring,
        ring_state,
        0,
        0,
        XHCI_TRB_TYPE_ENABLE_SLOT << 10,
    );
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(0, 0);
    unsafe { write_doorbell(doorbell_base, 0, dbell_value) };
    let (completion, slot_id) =
        wait_command_completion(mmio, rtsoff, event_ring, XHCI_TRB_TYPE_COMMAND_COMPLETION)?;
    if completion != XHCI_CC_SUCCESS || slot_id == 0 {
        platform::println!(
            "usb-driver: enable slot failed completion={} slot_id={}",
            completion,
            slot_id
        );
        return None;
    }
    Some(slot_id)
}

fn init_transfer_ring(ring: &DmaPage) {
    ring.zero();
    let trb_count = ring.len / size_of::<Trb>();
    let link_offset = (trb_count - 1) * size_of::<Trb>();
    ring.write_u64(link_offset, ring.phys);
    ring.write_u32(link_offset + 8, 0);
    ring.write_u32(
        link_offset + 12,
        (XHCI_TRB_TYPE_LINK << 10) | XHCI_TRB_TC | XHCI_TRB_CYCLE,
    );
}

fn write_dma_u64(base_virt: u64, offset: usize, value: u64) {
    // SAFETY: caller provides a writable mapped DMA virtual base and a valid in-page offset.
    unsafe { write_volatile((base_virt as *mut u8).add(offset) as *mut u64, value) }
}

fn write_dma_u32(base_virt: u64, offset: usize, value: u32) {
    // SAFETY: caller provides a writable mapped DMA virtual base and a valid in-page offset.
    unsafe { write_volatile((base_virt as *mut u8).add(offset) as *mut u32, value) }
}

fn zero_dma_range(base_virt: u64, len: usize) {
    // SAFETY: caller provides a writable mapped DMA virtual base for len bytes.
    unsafe { core::ptr::write_bytes(base_virt as *mut u8, 0, len) }
}

fn init_slot_contexts(
    dcbaa_virt: u64,
    hccparams1: u32,
    slot_id: u8,
    root_port: u8,
    speed_id: u32,
) -> Result<(DmaPage, DmaPage, DmaPage, TransferRingState), syscall::SysError> {
    let output_ctx = DmaPage::allocate(4096)?;
    let input_ctx = DmaPage::allocate(4096)?;
    let ep0_ring = DmaPage::allocate(4096)?;
    output_ctx.zero();
    input_ctx.zero();
    init_transfer_ring(&ep0_ring);

    let context_size = if (hccparams1 & XHCI_CTX_SIZE_64) != 0 {
        64usize
    } else {
        32usize
    };
    let ep0_max_packet_size = match speed_id {
        3 => 64u32,
        4 | 5 => 512u32,
        _ => 8u32,
    };
    let slot_ctx = context_size;
    let ep0_ctx = context_size * 2;

    write_dma_u64(dcbaa_virt, slot_id as usize * 8, output_ctx.phys);
    input_ctx.write_u32(XHCI_INPUT_CONTROL_DROP_FLAGS, 0);
    input_ctx.write_u32(XHCI_INPUT_CONTROL_ADD_FLAGS, 0x3);
    input_ctx.write_u32(XHCI_INPUT_CONTROL_CONFIG_VALUE, 0);
    input_ctx.write_u32(
        slot_ctx + XHCI_SLOT_CTX_DW0,
        ((speed_id & 0xF) << 20) | (1 << 27),
    );
    input_ctx.write_u32(
        slot_ctx + XHCI_SLOT_CTX_DW1,
        (root_port as u32) << 16,
    );
    input_ctx.write_u32(ep0_ctx + XHCI_EP_CTX_DW0, 0);
    input_ctx.write_u32(
        ep0_ctx + XHCI_EP_CTX_DW1,
        (3 << 1) | (XHCI_EP_TYPE_CONTROL_BIDIR << 3) | (ep0_max_packet_size << 16),
    );
    input_ctx.write_u64(ep0_ctx + XHCI_EP_CTX_TR_DEQUEUE_LO, ep0_ring.phys | 1);
    input_ctx.write_u32(ep0_ctx + XHCI_EP_CTX_DW4, 8);
    Ok((
        output_ctx,
        input_ctx,
        ep0_ring,
        TransferRingState {
            next_index: 0,
            cycle: XHCI_TRB_CYCLE,
        },
    ))
}

fn address_device(
    mmio: &MmioRegion,
    dboff: usize,
    rtsoff: usize,
    command_ring: &DmaPage,
    ring_state: &mut CommandRingState,
    event_ring: &DmaPage,
    input_ctx: &DmaPage,
    slot_id: u8,
) -> bool {
    queue_command_trb(
        command_ring,
        ring_state,
        input_ctx.phys,
        0,
        (XHCI_TRB_TYPE_ADDRESS_DEVICE << 10) | ((slot_id as u32) << 24),
    );
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(0, 0);
    unsafe { write_doorbell(doorbell_base, 0, dbell_value) };
    let Some((completion, completed_slot_id)) =
        wait_command_completion(mmio, rtsoff, event_ring, XHCI_TRB_TYPE_COMMAND_COMPLETION)
    else {
        return false;
    };
    if completion != XHCI_CC_SUCCESS || completed_slot_id != slot_id {
        platform::println!(
            "usb-driver: address device failed completion={} slot_id={}",
            completion,
            completed_slot_id
        );
        return false;
    }
    true
}

fn build_setup_packet(
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
) -> u64 {
    u64::from(request_type)
        | (u64::from(request) << 8)
        | (u64::from(value) << 16)
        | (u64::from(index) << 32)
        | (u64::from(length) << 48)
}

fn allocate_descriptor_page() -> Option<DmaPage> {
    let descriptor = DmaPage::allocate(4096).ok()?;
    descriptor.zero();
    Some(descriptor)
}

fn queue_descriptor_read(
    ep0_ring: &DmaPage,
    ring_state: &mut TransferRingState,
    descriptor: &DmaPage,
    descriptor_type: u16,
    length: u16,
) {
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        build_setup_packet(0x80, 0x06, descriptor_type << 8, 0, length),
        8,
        (XHCI_TRB_TYPE_SETUP_STAGE << 10) | XHCI_TRB_IDT | (3 << 16),
    );
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        descriptor.phys,
        length as u32,
        (XHCI_TRB_TYPE_DATA_STAGE << 10) | XHCI_TRB_DIR_IN,
    );
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        0,
        0,
        (XHCI_TRB_TYPE_STATUS_STAGE << 10) | XHCI_TRB_IOC,
    );
}

fn queue_control_no_data(
    ep0_ring: &DmaPage,
    ring_state: &mut TransferRingState,
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
) {
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        build_setup_packet(request_type, request, value, index, 0),
        8,
        (XHCI_TRB_TYPE_SETUP_STAGE << 10) | XHCI_TRB_IDT | (2 << 16),
    );
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        0,
        0,
        (XHCI_TRB_TYPE_STATUS_STAGE << 10) | XHCI_TRB_IOC | XHCI_TRB_DIR_IN,
    );
}

fn configure_interrupt_in_endpoint(
    ctx: &mut ConfigureEndpointCtx<'_>,
    ep: InterruptEndpointInfo,
) -> Option<(DmaPage, TransferRingState)> {
    let ep1_ring = DmaPage::allocate(4096).ok()?;
    ep1_ring.zero();
    init_transfer_ring(&ep1_ring);

    let context_size = if (ctx.hccparams1 & XHCI_CTX_SIZE_64) != 0 {
        64usize
    } else {
        32usize
    };
    let slot_ctx = context_size;
    let ep1_in_ctx = context_size * 3;
    let interval = ep.interval.saturating_sub(1).min(15) as u32;

    ctx.input_ctx.copy_from(ctx.output_ctx);
    ctx.input_ctx.write_u32(XHCI_INPUT_CONTROL_DROP_FLAGS, 0);
    ctx.input_ctx.write_u32(XHCI_INPUT_CONTROL_ADD_FLAGS, 0x9);
    ctx.input_ctx.write_u32(XHCI_INPUT_CONTROL_CONFIG_VALUE, 0);
    ctx.input_ctx.write_u32(
        slot_ctx + XHCI_SLOT_CTX_DW0,
        ((ctx.speed_id & 0xF) << 20) | (3 << 27),
    );
    ctx.input_ctx.write_u32(
        slot_ctx + XHCI_SLOT_CTX_DW1,
        (ctx.root_port as u32) << 16,
    );
    ctx.input_ctx.write_u32(ep1_in_ctx + XHCI_EP_CTX_DW0, interval << 16);
    ctx.input_ctx.write_u32(
        ep1_in_ctx + XHCI_EP_CTX_DW1,
        (3 << 1) | (XHCI_EP_TYPE_INTERRUPT_IN << 3) | ((ep.max_packet as u32) << 16),
    );
    ctx.input_ctx
        .write_u64(ep1_in_ctx + XHCI_EP_CTX_TR_DEQUEUE_LO, ep1_ring.phys | 1);
    ctx.input_ctx.write_u32(
        ep1_in_ctx + XHCI_EP_CTX_DW4,
        (ep.max_packet as u32) | ((ep.max_packet as u32) << 16),
    );

    queue_command_trb(
        ctx.command_ring,
        ctx.ring_state,
        ctx.input_ctx.phys,
        0,
        (XHCI_TRB_TYPE_CONFIGURE_ENDPOINT << 10) | ((ctx.slot_id as u32) << 24),
    );
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(0, 0);
    unsafe { write_doorbell(doorbell_base, 0, dbell_value) };
    let (completion, completed_slot_id) =
        wait_command_completion(ctx.mmio, ctx.rtsoff, ctx.event_ring, XHCI_TRB_TYPE_COMMAND_COMPLETION)?;
    if completion != XHCI_CC_SUCCESS || completed_slot_id != ctx.slot_id {
        platform::println!(
            "usb-driver: configure endpoint failed completion={} slot_id={}",
            completion,
            completed_slot_id
        );
        return None;
    }

    Some((
        ep1_ring,
        TransferRingState {
            next_index: 0,
            cycle: XHCI_TRB_CYCLE,
        },
    ))
}

fn queue_interrupt_in_transfer(
    mmio: &MmioRegion,
    rtsoff: usize,
    event_ring: &DmaPage,
    slot_id: u8,
    ep1_ring: &DmaPage,
    ring_state: &mut TransferRingState,
    ep: InterruptEndpointInfo,
) -> Option<DmaPage> {
    let report = DmaPage::allocate(4096).ok()?;
    let dci = endpoint_dci(ep.address);
    for attempt in 0..16 {
        report.zero();
        clear_event_trbs(event_ring);
        queue_transfer_trb(
            ep1_ring,
            ring_state,
            report.phys,
            ep.max_packet as u32,
            (XHCI_TRB_TYPE_NORMAL << 10) | XHCI_TRB_ISP | XHCI_TRB_IOC,
        );
        let doorbell_base = current_doorbell_base();
        let dbell_value = doorbell_value(dci, 0);
        unsafe { write_doorbell(doorbell_base, slot_id as u32, dbell_value) };
        let Some(completion) = wait_transfer_event(mmio, rtsoff, event_ring, slot_id) else {
            continue;
        };
        if completion == XHCI_CC_SUCCESS {
            return Some(report);
        }
    }
    None
}

fn hid_set_idle(
    mmio: &MmioRegion,
    rtsoff: usize,
    event_ring: &DmaPage,
    slot_id: u8,
    ep0_ring: &DmaPage,
    ring_state: &mut TransferRingState,
    interface_number: u8,
) -> bool {
    clear_event_trbs(event_ring);
    queue_control_no_data(
        ep0_ring,
        ring_state,
        0x21,
        HID_REQ_SET_IDLE,
        0,
        interface_number as u16,
    );
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(1, 0);
    unsafe { write_doorbell(doorbell_base, slot_id as u32, dbell_value) };
    matches!(
        wait_transfer_event(mmio, rtsoff, event_ring, slot_id),
        Some(XHCI_CC_SUCCESS)
    )
}

fn hid_set_protocol(
    mmio: &MmioRegion,
    rtsoff: usize,
    event_ring: &DmaPage,
    slot_id: u8,
    ep0_ring: &DmaPage,
    ring_state: &mut TransferRingState,
    interface_number: u8,
    protocol: u16,
) -> bool {
    clear_event_trbs(event_ring);
    queue_control_no_data(
        ep0_ring,
        ring_state,
        0x21,
        HID_REQ_SET_PROTOCOL,
        protocol,
        interface_number as u16,
    );
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(1, 0);
    unsafe { write_doorbell(doorbell_base, slot_id as u32, dbell_value) };
    matches!(
        wait_transfer_event(mmio, rtsoff, event_ring, slot_id),
        Some(XHCI_CC_SUCCESS)
    )
}

fn log_hid_input_report(report: &DmaPage, ep: InterruptEndpointInfo) {
    let packet_len = core::cmp::min(ep.max_packet as usize, report.len);
    if packet_len == 0 {
        return;
    }

    let mut line = [0u8; 3 * 16];
    let preview_len = core::cmp::min(packet_len, 16);
    for i in 0..preview_len {
        let byte = report.read_u8(i);
        let hi = byte >> 4;
        let lo = byte & 0x0f;
        line[i * 3] = if hi < 10 { b'0' + hi } else { b'a' + (hi - 10) };
        line[i * 3 + 1] = if lo < 10 { b'0' + lo } else { b'a' + (lo - 10) };
        line[i * 3 + 2] = if i + 1 == preview_len { 0 } else { b' ' };
    }
    if let Ok(text) = core::str::from_utf8(&line[..preview_len * 3 - 1]) {
        platform::println!("usb-driver: hid report bytes={}", text);
    }

    if packet_len >= 5 {
        let buttons = report.read_u8(0) & 0x1f;
        let x = u16::from_le_bytes([report.read_u8(1), report.read_u8(2)]);
        let y = u16::from_le_bytes([report.read_u8(3), report.read_u8(4)]);
        let wheel = if packet_len >= 6 {
            report.read_u8(5) as i8
        } else {
            0
        };
        platform::println!(
            "usb-driver: hid pointer buttons=0x{:02x} x={} y={} wheel={}",
            buttons,
            x,
            y,
            wheel
        );
    }
}

fn read_device_descriptor(
    mmio: &MmioRegion,
    dboff: usize,
    rtsoff: usize,
    event_ring: &DmaPage,
    slot_id: u8,
    ep0_ring: &DmaPage,
    ring_state: &mut TransferRingState,
) -> Option<(u16, u16)> {
    let descriptor = allocate_descriptor_page()?;
    clear_event_trbs(event_ring);
    queue_descriptor_read(ep0_ring, ring_state, &descriptor, USB_DESC_TYPE_DEVICE, 18);
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(1, 0);
    unsafe { write_doorbell(doorbell_base, slot_id as u32, dbell_value) };
    let completion = wait_transfer_event(mmio, rtsoff, event_ring, slot_id)?;
    if completion != XHCI_CC_SUCCESS {
        platform::println!(
            "usb-driver: get device descriptor transfer failed completion={}",
            completion
        );
        return None;
    }

    let vendor = descriptor.read_u32(8) as u16;
    let product = (descriptor.read_u32(8) >> 16) as u16;
    Some((vendor, product))
}

fn read_report_descriptor(
    mmio: &MmioRegion,
    dboff: usize,
    rtsoff: usize,
    event_ring: &DmaPage,
    slot_id: u8,
    ep0_ring: &DmaPage,
    ring_state: &mut TransferRingState,
    interface_number: u8,
    report_len: u16,
) -> bool {
    let descriptor = match allocate_descriptor_page() {
        Some(page) => page,
        None => return false,
    };
    let transfer_length = report_len.min(descriptor.len as u16);
    clear_event_trbs(event_ring);
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        build_setup_packet(0x81, 0x06, USB_DESC_TYPE_REPORT << 8, interface_number as u16, transfer_length),
        8,
        (XHCI_TRB_TYPE_SETUP_STAGE << 10) | XHCI_TRB_IDT | (3 << 16),
    );
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        descriptor.phys,
        transfer_length as u32,
        (XHCI_TRB_TYPE_DATA_STAGE << 10) | XHCI_TRB_DIR_IN,
    );
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        0,
        0,
        (XHCI_TRB_TYPE_STATUS_STAGE << 10) | XHCI_TRB_IOC,
    );
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(1, 0);
    unsafe { write_doorbell(doorbell_base, slot_id as u32, dbell_value) };
    let Some(completion) = wait_transfer_event(mmio, rtsoff, event_ring, slot_id) else {
        return false;
    };
    if completion != XHCI_CC_SUCCESS {
        platform::println!(
            "usb-driver: get report descriptor failed completion={}",
            completion
        );
        return false;
    }

    let preview_len = core::cmp::min(16usize, transfer_length as usize);
    let mut line = [0u8; 3 * 16];
    for i in 0..preview_len {
        let byte = {
            // SAFETY: descriptor page is valid DMA-backed memory and i is range-checked.
            unsafe { read_volatile(descriptor.ptr().add(i) as *const u8) }
        };
        let hi = byte >> 4;
        let lo = byte & 0x0f;
        line[i * 3] = if hi < 10 { b'0' + hi } else { b'a' + (hi - 10) };
        line[i * 3 + 1] = if lo < 10 { b'0' + lo } else { b'a' + (lo - 10) };
        line[i * 3 + 2] = if i + 1 == preview_len { 0 } else { b' ' };
    }
    if let Ok(text) = core::str::from_utf8(&line[..preview_len * 3 - 1]) {
        platform::println!("usb-driver: report descriptor bytes={}", text);
    }
    true
}

fn read_configuration_descriptor(
    mmio: &MmioRegion,
    dboff: usize,
    rtsoff: usize,
    event_ring: &DmaPage,
    slot_id: u8,
    ep0_ring: &DmaPage,
    ring_state: &mut TransferRingState,
) -> Option<UsbConfigurationInfo> {
    let descriptor = match allocate_descriptor_page() {
        Some(page) => page,
        None => return None,
    };

    clear_event_trbs(event_ring);
    queue_descriptor_read(
        ep0_ring,
        ring_state,
        &descriptor,
        USB_DESC_TYPE_CONFIGURATION,
        9,
    );
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(1, 0);
    unsafe { write_doorbell(doorbell_base, slot_id as u32, dbell_value) };
    let Some(completion) = wait_transfer_event(mmio, rtsoff, event_ring, slot_id) else {
        return None;
    };
    if completion != XHCI_CC_SUCCESS {
        platform::println!(
            "usb-driver: get config header failed completion={}",
            completion
        );
        return None;
    }

    let total_length = ((descriptor.read_u32(0) >> 16) & 0xffff) as u16;
    let transfer_length = total_length.min(descriptor.len as u16);
    descriptor.zero();
    clear_event_trbs(event_ring);
    queue_descriptor_read(
        ep0_ring,
        ring_state,
        &descriptor,
        USB_DESC_TYPE_CONFIGURATION,
        transfer_length,
    );
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(1, 0);
    unsafe { write_doorbell(doorbell_base, slot_id as u32, dbell_value) };
    let Some(completion) = wait_transfer_event(mmio, rtsoff, event_ring, slot_id) else {
        return None;
    };
    if completion != XHCI_CC_SUCCESS {
        platform::println!(
            "usb-driver: get config descriptor failed completion={}",
            completion
        );
        return None;
    }

    let config_value = ((descriptor.read_u32(4) >> 8) & 0xff) as u8;
    let interface_count = descriptor.read_u32(4) as u8;
    let mut interface_number = 0u8;
    let mut interface_class = 0u8;
    let mut interface_subclass = 0u8;
    let mut interface_protocol = 0u8;
    let mut report_descriptor_len = 0u16;
    let mut interrupt_in = None;
    let bytes = descriptor.ptr();
    let mut offset = 0usize;
    while offset + 2 <= transfer_length as usize {
        // SAFETY: descriptor buffer is DMA-backed page owned by this process; bounds checked above.
        let len = unsafe { read_volatile(bytes.add(offset) as *const u8) } as usize;
        if len < 2 || offset + len > transfer_length as usize {
            break;
        }
        // SAFETY: descriptor buffer is DMA-backed page owned by this process; bounds checked above.
        let dtype = unsafe { read_volatile(bytes.add(offset + 1) as *const u8) };
        match dtype {
            USB_DESC_TYPE_INTERFACE if len >= 9 => {
                // SAFETY: interface descriptor fields are inside validated descriptor bounds.
                let iface_num = unsafe { read_volatile(bytes.add(offset + 2) as *const u8) };
                // SAFETY: interface descriptor fields are inside validated descriptor bounds.
                let alt = unsafe { read_volatile(bytes.add(offset + 3) as *const u8) };
                // SAFETY: interface descriptor fields are inside validated descriptor bounds.
                let eps = unsafe { read_volatile(bytes.add(offset + 4) as *const u8) };
                // SAFETY: interface descriptor fields are inside validated descriptor bounds.
                let class = unsafe { read_volatile(bytes.add(offset + 5) as *const u8) };
                // SAFETY: interface descriptor fields are inside validated descriptor bounds.
                let subclass = unsafe { read_volatile(bytes.add(offset + 6) as *const u8) };
                // SAFETY: interface descriptor fields are inside validated descriptor bounds.
                let proto = unsafe { read_volatile(bytes.add(offset + 7) as *const u8) };
                interface_number = iface_num;
                interface_class = class;
                interface_subclass = subclass;
                interface_protocol = proto;
            }
            USB_DESC_TYPE_ENDPOINT if len >= 7 => {
                // SAFETY: endpoint descriptor fields are inside validated descriptor bounds.
                let addr = unsafe { read_volatile(bytes.add(offset + 2) as *const u8) };
                // SAFETY: endpoint descriptor fields are inside validated descriptor bounds.
                let attrs = unsafe { read_volatile(bytes.add(offset + 3) as *const u8) };
                // SAFETY: endpoint descriptor fields are inside validated descriptor bounds.
                let max_packet = unsafe { read_volatile(bytes.add(offset + 4) as *const u16) };
                // SAFETY: endpoint descriptor fields are inside validated descriptor bounds.
                let interval = unsafe { read_volatile(bytes.add(offset + 6) as *const u8) };
                if (addr & 0x80) != 0 && (attrs & 0x3) == 0x3 && interrupt_in.is_none() {
                    interrupt_in = Some(InterruptEndpointInfo {
                        address: addr,
                        max_packet,
                        interval,
                    });
                }
            }
            USB_DESC_TYPE_HID if len >= 9 => {
                // SAFETY: HID descriptor fields are inside validated descriptor bounds.
                let report_len =
                    unsafe { read_volatile(bytes.add(offset + 7) as *const u16) };
                report_descriptor_len = report_len;
            }
            _ => {}
        }
        offset += len;
    }

    Some(UsbConfigurationInfo {
        config_value,
        interface_number,
        interface_class,
        interface_subclass,
        interface_protocol,
        report_descriptor_len,
        interrupt_in,
    })
}

fn set_configuration(
    mmio: &MmioRegion,
    dboff: usize,
    rtsoff: usize,
    event_ring: &DmaPage,
    slot_id: u8,
    ep0_ring: &DmaPage,
    ring_state: &mut TransferRingState,
    config_value: u8,
) -> bool {
    clear_event_trbs(event_ring);
    queue_control_no_data(ep0_ring, ring_state, 0x00, 0x09, config_value as u16, 0);
    let doorbell_base = current_doorbell_base();
    let dbell_value = doorbell_value(1, 0);
    unsafe { write_doorbell(doorbell_base, slot_id as u32, dbell_value) };
    let Some(completion) = wait_transfer_event(mmio, rtsoff, event_ring, slot_id) else {
        return false;
    };
    if completion != XHCI_CC_SUCCESS {
        platform::println!(
            "usb-driver: set configuration failed completion={} value={}",
            completion,
            config_value
        );
        return false;
    }
    true
}

fn reset_port(mmio: &MmioRegion, port_offset: usize, port_index: usize) -> Option<u32> {
    let initial = mmio.read_u32(port_offset);
    if (initial & XHCI_PORTSC_CCS) == 0 || (initial & XHCI_PORTSC_OCA) != 0 {
        return None;
    }
    if (initial & XHCI_PORTSC_PED) == 0 {
        mmio.write_u32(
            port_offset,
            (initial & !(XHCI_PORTSC_PR | XHCI_PORTSC_CHANGE_BITS)) | XHCI_PORTSC_PR | XHCI_PORTSC_CHANGE_BITS,
        );
        if !wait_until("xhci port reset", 100_000, || {
            let value = mmio.read_u32(port_offset);
            (value & XHCI_PORTSC_PR) == 0 && (value & XHCI_PORTSC_PED) != 0
        }) {
            platform::println!("usb-driver: port{} reset failed", port_index + 1);
            return None;
        }
    }
    Some(mmio.read_u32(port_offset))
}

fn port_in(port: u16, width: u64) -> Result<u64, syscall::SysError> {
    syscall::call2(syscall::SyscallNumber::PortIn, port as u64, width)
}

fn port_out(port: u16, value: u64, width: u64) -> Result<u64, syscall::SysError> {
    syscall::call3(syscall::SyscallNumber::PortOut, port as u64, value, width)
}

fn pci_config_address(loc: PciLocation, offset: u8) -> u32 {
    0x8000_0000
        | ((loc.bus as u32) << 16)
        | ((loc.device as u32) << 11)
        | ((loc.function as u32) << 8)
        | ((offset as u32) & 0xFC)
}

fn pci_read_u32(loc: PciLocation, offset: u8) -> Option<u32> {
    let addr = pci_config_address(loc, offset);
    if port_out(PCI_CFG_ADDR, addr as u64, 4).is_err() {
        // SAFETY: best-effort single log for diagnosis in single-threaded startup path.
        unsafe {
            if !PCI_CONFIG_IO_FAILED_LOGGED {
                PCI_CONFIG_IO_FAILED_LOGGED = true;
                platform::println!("usb-driver: pci config io denied");
            }
        }
        return None;
    }
    let value = match port_in(PCI_CFG_DATA, 4) {
        Ok(v) => v,
        Err(_) => {
            // SAFETY: best-effort single log for diagnosis in single-threaded startup path.
            unsafe {
                if !PCI_CONFIG_IO_FAILED_LOGGED {
                    PCI_CONFIG_IO_FAILED_LOGGED = true;
                    platform::println!("usb-driver: pci config io denied");
                }
            }
            return None;
        }
    };
    Some(value as u32)
}

fn pci_write_u32(loc: PciLocation, offset: u8, value: u32) -> bool {
    let addr = pci_config_address(loc, offset);
    port_out(PCI_CFG_ADDR, addr as u64, 4).is_ok()
        && port_out(PCI_CFG_DATA, value as u64, 4).is_ok()
}

fn pci_read_u16(loc: PciLocation, offset: u8) -> Option<u16> {
    let aligned = offset & !0x3;
    let shift = ((offset & 0x2) as u32) * 8;
    pci_read_u32(loc, aligned).map(|v| ((v >> shift) & 0xFFFF) as u16)
}

fn pci_write_u16(loc: PciLocation, offset: u8, value: u16) -> bool {
    let aligned = offset & !0x3;
    let shift = ((offset & 0x2) as u32) * 8;
    let Some(mut current) = pci_read_u32(loc, aligned) else {
        return false;
    };
    current &= !(0xFFFFu32 << shift);
    current |= (value as u32) << shift;
    pci_write_u32(loc, aligned, current)
}

fn pci_read_u8(loc: PciLocation, offset: u8) -> Option<u8> {
    let aligned = offset & !0x3;
    let shift = ((offset & 0x3) as u32) * 8;
    pci_read_u32(loc, aligned).map(|v| ((v >> shift) & 0xFF) as u8)
}

fn pci_command_enable_memory_and_bus_master(loc: PciLocation) {
    if let Some(command) = pci_read_u16(loc, 0x04) {
        let updated = command | 0x0006;
        let _ = pci_write_u16(loc, 0x04, updated);
    }
}

fn probe_mem_bar(loc: PciLocation, bar_idx: u8) -> Option<XhciBar> {
    if bar_idx >= 6 {
        return None;
    }
    let offset = 0x10 + bar_idx * 4;
    let original_low = pci_read_u32(loc, offset)?;
    if (original_low & 0x1) != 0 {
        return None;
    }

    let bar_type = (original_low >> 1) & 0x3;
    let is_64 = bar_type == 0x2;
    let original_high = if is_64 {
        if bar_idx + 1 >= 6 {
            return None;
        }
        pci_read_u32(loc, offset + 4)?
    } else {
        0
    };

    let base = if is_64 {
        ((original_high as u64) << 32) | ((original_low & 0xffff_fff0) as u64)
    } else {
        (original_low & 0xffff_fff0) as u64
    };
    if base == 0 {
        return None;
    }

    Some(XhciBar {
        phys_base: base,
        size: 0x1_0000,
    })
}

fn find_xhci_bar(loc: PciLocation) -> Option<XhciBar> {
    let mut bar_idx = 0u8;
    while bar_idx < 6 {
        let offset = 0x10 + bar_idx * 4;
        let value = pci_read_u32(loc, offset)?;
        if (value & 0x1) == 0 && let Some(bar) = probe_mem_bar(loc, bar_idx) {
            return Some(bar);
        }
        let bar_type = (value >> 1) & 0x3;
        bar_idx += if bar_type == 0x2 { 2 } else { 1 };
    }
    None
}

fn log_pci_bars(loc: PciLocation) {
    for bar_idx in 0u8..6 {
        let offset = 0x10 + bar_idx * 4;
        let _ = pci_read_u32(loc, offset);
    }
}

fn xhci_port_speed_name(speed_id: u32) -> &'static str {
    match speed_id {
        0 => "unknown",
        1 => "full",
        2 => "low",
        3 => "high",
        4 => "super",
        5 => "super+",
        _ => "reserved",
    }
}

fn looks_like_xhci(mmio: &MmioRegion) -> bool {
    let cap_length = mmio.read_u8(XHCI_CAP_CAPLENGTH) as usize;
    let hcsparams1 = mmio.read_u32(XHCI_CAP_HCSPARAMS1);
    let max_slots = hcsparams1 & 0xff;
    cap_length >= 0x20 && max_slots > 0 && hcsparams1 != 0
}

fn enumerate_xhci_controller(loc: PciLocation, bar: XhciBar, vendor: u16, device: u16) {
    pci_command_enable_memory_and_bus_master(loc);

    let map_len = core::cmp::min(bar.size, 0x10000);
    let mmio = match MmioRegion::map(bar.phys_base, map_len) {
        Ok(region) => region,
        Err(_) => {
            platform::println!(
                "usb-driver: failed to map xHCI MMIO bar phys=0x{:016x} size=0x{:x}",
                bar.phys_base,
                bar.size
            );
            return;
        }
    };

    let cap_length = mmio.read_u8(XHCI_CAP_CAPLENGTH) as usize;
    let hci_version = mmio.read_u16(XHCI_CAP_HCIVERSION);
    let hcsparams1 = mmio.read_u32(XHCI_CAP_HCSPARAMS1);
    let hccparams1 = mmio.read_u32(XHCI_CAP_HCCPARAMS1);
    let dboff = (mmio.read_u32(XHCI_CAP_DBOFF) & !0x3) as usize;
    let rtsoff = (mmio.read_u32(XHCI_CAP_RTSOFF) & !0x1f) as usize;
    // SAFETY: single-threaded enumeration path initializes global doorbell base once per controller.
    unsafe {
        XHCI_DOORBELL_BASE = mmio.virt_base + dboff;
    }
    let max_slots = hcsparams1 & 0xff;
    let max_intrs = (hcsparams1 >> 8) & 0x7ff;
    let max_ports = (hcsparams1 >> 24) & 0xff;

    let usbcmd = mmio.read_u32(cap_length + XHCI_OP_USBCMD);
    let usbsts = mmio.read_u32(cap_length + XHCI_OP_USBSTS);
    let pagesize = mmio.read_u32(cap_length + XHCI_OP_PAGESIZE);
    let config = mmio.read_u32(cap_length + XHCI_OP_CONFIG);

    platform::println!(
        "usb-driver: PCI USB controller bus={:02x} dev={:02x} func={} vendor=0x{:04x} device=0x{:04x} class=0x0c subclass=0x03 prog_if=0x{:02x} header=0x{:02x}",
        loc.bus,
        loc.device,
        loc.function,
        vendor,
        device,
        XHCI_PROG_IF,
        pci_read_u8(loc, 0x0E).unwrap_or(0),
    );
    platform::println!(
        "usb-driver: xhci controller bus={:02x} dev={:02x} func={} vendor=0x{:04x} device=0x{:04x} mmio_base=0x{:016x} mmio_size=0x{:x}",
        loc.bus,
        loc.device,
        loc.function,
        vendor,
        device,
        bar.phys_base,
        bar.size
    );
    platform::println!(
        "usb-driver: xhci caplen=0x{:02x} hci=0x{:04x} slots={} intrs={} ports={} hcc=0x{:08x} dboff=0x{:x} rtsoff=0x{:x}",
        cap_length,
        hci_version,
        max_slots,
        max_intrs,
        max_ports,
        hccparams1,
        dboff,
        rtsoff
    );
    if !xhci_stop_and_reset(&mmio, cap_length) {
        platform::println!("usb-driver: xhci reset failed");
        return;
    }
    let dcbaa = match DmaPage::allocate(4096) {
        Ok(page) => page,
        Err(_) => {
            platform::println!("usb-driver: xhci ring initialization failed");
            return;
        }
    };
    let command_ring = match DmaPage::allocate(4096) {
        Ok(page) => page,
        Err(_) => {
            platform::println!("usb-driver: xhci ring initialization failed");
            return;
        }
    };
    let event_ring = match DmaPage::allocate(4096) {
        Ok(page) => page,
        Err(_) => {
            platform::println!("usb-driver: xhci ring initialization failed");
            return;
        }
    };
    let erst = match DmaPage::allocate(4096) {
        Ok(page) => page,
        Err(_) => {
            platform::println!("usb-driver: xhci ring initialization failed");
            return;
        }
    };

    dcbaa.zero();
    command_ring.zero();
    event_ring.zero();
    erst.zero();

    let trb_count = command_ring.len / size_of::<Trb>();
    let link_offset = (trb_count - 1) * size_of::<Trb>();
    command_ring.write_u64(link_offset, command_ring.phys);
    command_ring.write_u32(link_offset + 8, 0);
    command_ring.write_u32(
        link_offset + 12,
        (XHCI_TRB_TYPE_LINK << 10) | XHCI_TRB_TC | XHCI_TRB_CYCLE,
    );

    erst.write_u64(0, event_ring.phys);
    erst.write_u32(8, (event_ring.len / size_of::<Trb>()) as u32);
    erst.write_u32(12, 0);

    mmio.write_u64(cap_length + XHCI_OP_DCBAAP, dcbaa.phys);
    mmio.write_u64(cap_length + XHCI_OP_CRCR, command_ring.phys | 1);
    mmio.write_u32(cap_length + XHCI_OP_DNCTRL, 0);
    mmio.write_u32(cap_length + XHCI_OP_CONFIG, 1);

    let ir0 = rtsoff + XHCI_RT_IR0;
    mmio.write_u32(ir0 + XHCI_IR_IMAN, 0);
    mmio.write_u32(ir0 + XHCI_IR_IMOD, 0);
    mmio.write_u32(ir0 + XHCI_IR_ERSTSZ, 1);
    mmio.write_u64(ir0 + XHCI_IR_ERSTBA, erst.phys);
    mmio.write_u64(ir0 + XHCI_IR_ERDP, event_ring.phys);

    let command_ring_state = core::ptr::addr_of_mut!(XHCI_COMMAND_RING_STATE);
    if !xhci_start(&mmio, cap_length) {
        platform::println!("usb-driver: xhci start failed");
        return;
    }
    let mut selected_port = None;
    for port_index in 0..max_ports as usize {
        let offset = cap_length + XHCI_OP_PORTSC_BASE + port_index * XHCI_OP_PORTSC_STRIDE;
        if offset + 4 > mmio.len {
            break;
        }
        let portsc = reset_port(&mmio, offset, port_index).unwrap_or_else(|| mmio.read_u32(offset));
        let connected = (portsc & 0x1) != 0;
        let enabled = (portsc & 0x2) != 0;
        let over_current = (portsc & 0x8) != 0;
        let reset = (portsc & 0x10) != 0;
        let power = (portsc & 0x200) != 0;
        let speed_id = (portsc >> 10) & 0xf;
        if connected && enabled && selected_port.is_none() {
            selected_port = Some((port_index as u8 + 1, speed_id));
        }
    }

    if let Some((root_port, speed_id)) = selected_port {
        if let Some(slot_id) = enable_slot(
            &mmio,
            dboff,
            rtsoff,
            &command_ring,
            unsafe { &mut *command_ring_state },
            &event_ring,
        ) {
            {
                let output_ctx = match DmaPage::allocate(4096) {
                    Ok(page) => page,
                    Err(_) => {
                        platform::println!("usb-driver: slot context initialization failed");
                        return;
                    }
                };
                let input_ctx = match DmaPage::allocate(4096) {
                    Ok(page) => page,
                    Err(_) => {
                        platform::println!("usb-driver: slot context initialization failed");
                        return;
                    }
                };
                let ep0_ring = match DmaPage::allocate(4096) {
                    Ok(page) => page,
                    Err(_) => {
                        platform::println!("usb-driver: slot context initialization failed");
                        return;
                    }
                };
                let output_ctx_virt = output_ctx.virt;
                let output_ctx_phys = output_ctx.phys;
                let output_ctx_len = output_ctx.len;
                let input_ctx_virt = input_ctx.virt;
                let input_ctx_phys = input_ctx.phys;
                let input_ctx_len = input_ctx.len;
                let ep0_ring_virt = ep0_ring.virt;
                let ep0_ring_phys = ep0_ring.phys;
                let ep0_ring_len = ep0_ring.len;

                zero_dma_range(output_ctx_virt, output_ctx_len);
                zero_dma_range(input_ctx_virt, input_ctx_len);
                zero_dma_range(ep0_ring_virt, ep0_ring_len);
                let trb_count = ep0_ring_len / size_of::<Trb>();
                let link_offset = (trb_count - 1) * size_of::<Trb>();
                write_dma_u64(ep0_ring_virt, link_offset, ep0_ring_phys);
                write_dma_u32(ep0_ring_virt, link_offset + 8, 0);
                write_dma_u32(
                    ep0_ring_virt,
                    link_offset + 12,
                    (XHCI_TRB_TYPE_LINK << 10) | XHCI_TRB_TC | XHCI_TRB_CYCLE,
                );

                let context_size = if (hccparams1 & XHCI_CTX_SIZE_64) != 0 {
                    64usize
                } else {
                    32usize
                };
                let ep0_max_packet_size = match speed_id {
                    3 => 64u32,
                    4 | 5 => 512u32,
                    _ => 8u32,
                };
                let slot_ctx = context_size;
                let ep0_ctx = context_size * 2;

                write_dma_u64(dcbaa.virt, slot_id as usize * 8, output_ctx_phys);
                write_dma_u32(input_ctx_virt, XHCI_INPUT_CONTROL_DROP_FLAGS, 0);
                write_dma_u32(input_ctx_virt, XHCI_INPUT_CONTROL_ADD_FLAGS, 0x3);
                write_dma_u32(input_ctx_virt, XHCI_INPUT_CONTROL_CONFIG_VALUE, 0);
                write_dma_u32(
                    input_ctx_virt,
                    slot_ctx + XHCI_SLOT_CTX_DW0,
                    ((speed_id & 0xF) << 20) | (1 << 27),
                );
                write_dma_u32(
                    input_ctx_virt,
                    slot_ctx + XHCI_SLOT_CTX_DW1,
                    (root_port as u32) << 16,
                );
                write_dma_u32(input_ctx_virt, ep0_ctx + XHCI_EP_CTX_DW0, 0);
                write_dma_u32(
                    input_ctx_virt,
                    ep0_ctx + XHCI_EP_CTX_DW1,
                    (3 << 1) | (XHCI_EP_TYPE_CONTROL_BIDIR << 3) | (ep0_max_packet_size << 16),
                );
                write_dma_u64(input_ctx_virt, ep0_ctx + XHCI_EP_CTX_TR_DEQUEUE_LO, ep0_ring_phys | 1);
                write_dma_u32(input_ctx_virt, ep0_ctx + XHCI_EP_CTX_DW4, 8);
                let mut ep0_ring_state = TransferRingState {
                    next_index: 0,
                    cycle: XHCI_TRB_CYCLE,
                };

                    if address_device(
                        &mmio,
                        dboff,
                        rtsoff,
                        &command_ring,
                        unsafe { &mut *command_ring_state },
                        &event_ring,
                        &input_ctx,
                        slot_id,
                    ) {
                        if let Some((vendor_id, product_id)) = read_device_descriptor(
                            &mmio,
                            dboff,
                            rtsoff,
                            &event_ring,
                            slot_id,
                            &ep0_ring,
                            &mut ep0_ring_state,
                        ) {
                            platform::println!(
                                "usb-driver: device descriptor vendor=0x{:04x} product=0x{:04x}",
                                vendor_id,
                                product_id
                            );
                            if let Some(config) = read_configuration_descriptor(
                                &mmio,
                                dboff,
                                rtsoff,
                                &event_ring,
                                slot_id,
                                &ep0_ring,
                                &mut ep0_ring_state,
                            ) {
                                if set_configuration(
                                    &mmio,
                                    dboff,
                                    rtsoff,
                                    &event_ring,
                                    slot_id,
                                    &ep0_ring,
                                    &mut ep0_ring_state,
                                    config.config_value,
                                ) {
                                    if let Some(ep) = config.interrupt_in {
                                        let idle_ok = hid_set_idle(
                                            &mmio,
                                            rtsoff,
                                            &event_ring,
                                            slot_id,
                                            &ep0_ring,
                                            &mut ep0_ring_state,
                                            config.interface_number,
                                        );
                                        let protocol_ok = hid_set_protocol(
                                            &mmio,
                                            rtsoff,
                                            &event_ring,
                                            slot_id,
                                            &ep0_ring,
                                            &mut ep0_ring_state,
                                            config.interface_number,
                                            1,
                                        );
                                        if let Some((ep1_ring, mut ep1_ring_state)) =
                                            configure_interrupt_in_endpoint(
                                                &mut ConfigureEndpointCtx {
                                                    mmio: &mmio,
                                                    rtsoff,
                                                    command_ring: &command_ring,
                                                    ring_state: unsafe { &mut *command_ring_state },
                                                    event_ring: &event_ring,
                                                    output_ctx: &output_ctx,
                                                    input_ctx: &input_ctx,
                                                    slot_id,
                                                    hccparams1,
                                                    root_port,
                                                    speed_id,
                                                },
                                                ep,
                                            )
                                        {
                                            loop {
                                                if let Some(report) = queue_interrupt_in_transfer(
                                                    &mmio,
                                                    rtsoff,
                                                    &event_ring,
                                                    slot_id,
                                                    &ep1_ring,
                                                    &mut ep1_ring_state,
                                                    ep,
                                                ) {
                                                    log_hid_input_report(&report, ep);
                                                }
                                                platform::thread::yield_now();
                                            }
                                        } else {
                                            platform::println!(
                                                "usb-driver: configure interrupt endpoint failed"
                                            );
                                        }
                                    }
                                if config.report_descriptor_len != 0 {
                                    let _ = read_report_descriptor(
                                        &mmio,
                                        dboff,
                                        rtsoff,
                                        &event_ring,
                                        slot_id,
                                            &ep0_ring,
                                            &mut ep0_ring_state,
                                            config.interface_number,
                                            config.report_descriptor_len,
                                        );
                                    }
                                } else {
                                    platform::println!(
                                        "usb-driver: set configuration failed"
                                    );
                                }
                            } else {
                                platform::println!(
                                    "usb-driver: configuration descriptor read failed"
                                );
                            }
                        } else {
                            platform::println!("usb-driver: device descriptor read failed");
                        }
                    } else {
                        platform::println!("usb-driver: address device timed out or failed");
                    }
            }
        } else {
            platform::println!("usb-driver: enable slot timed out or failed");
        }
    } else {
        platform::println!("usb-driver: no enabled usb ports after reset");
    }
}

fn pci_scan() {
    let mut found = 0usize;
    for bus in 0u8..=255 {
        for device in 0u8..32 {
            for function in 0u8..8 {
                let loc = PciLocation { bus, device, function };
                let Some(vendor_device) = pci_read_u32(loc, 0x00) else {
                    continue;
                };
                let vendor = (vendor_device & 0xFFFF) as u16;
                if vendor == 0xFFFF {
                    continue;
                }
                let device_id = (vendor_device >> 16) as u16;
                let Some(class_reg) = pci_read_u32(loc, 0x08) else {
                    continue;
                };
                let class = (class_reg >> 24) as u8;
                let subclass = (class_reg >> 16) as u8;
                let prog_if = (class_reg >> 8) as u8;
                if class != 0x0C || subclass != 0x03 {
                    continue;
                }

                log_pci_bars(loc);

                let Some(bar) = find_xhci_bar(loc) else {
                    continue;
                };

                let mut spec = DeviceSpec::new(
                    format!("/pci/{:02x}:{:02x}.{}", bus, device, function),
                    "usb-controller",
                    DeviceBus::Pci,
                    DeviceClass::Usb,
                );
                spec.vendor_id = Some(vendor as u32);
                spec.device_id = Some(device_id as u32);
                spec.revision = pci_read_u8(loc, 0x08);
                spec.properties
                    .insert("pci.bus".into(), DeviceProperty::U32(bus as u32));
                spec.properties
                    .insert("pci.device".into(), DeviceProperty::U32(device as u32));
                spec.properties
                    .insert("pci.function".into(), DeviceProperty::U32(function as u32));
                spec.properties
                    .insert("pci.mmio_base".into(), DeviceProperty::U64(bar.phys_base));
                spec.properties
                    .insert("pci.mmio_size".into(), DeviceProperty::U64(bar.size));
                spec.properties
                    .insert("pci.prog_if".into(), DeviceProperty::U32(prog_if as u32));

                let dev = register_device(spec);
                let _ = UsbDriver::start(dev, PlugKitResources::empty());
                found += 1;
            }
        }
    }

    if found == 0 {
        platform::println!("usb-driver: no xHCI USB controller found");
    }
}

struct UsbDriver;

impl PlugKitDriver for UsbDriver {
    fn probe(device: &PlugKitDevice) -> ProbeResult {
        if device.bus() == DeviceBus::Pci && device.class() == DeviceClass::Usb {
            ProbeResult::Match { score: 100 }
        } else {
            ProbeResult::Reject
        }
    }

    fn start(device: PlugKitDevice, _resources: PlugKitResources) -> PlugKitResult<()> {
        let bus = match device.property("pci.bus")? {
            Some(DeviceProperty::U32(v)) => v as u8,
            _ => return Err(PlugKitError::InvalidHandle),
        };
        let dev = match device.property("pci.device")? {
            Some(DeviceProperty::U32(v)) => v as u8,
            _ => return Err(PlugKitError::InvalidHandle),
        };
        let func = match device.property("pci.function")? {
            Some(DeviceProperty::U32(v)) => v as u8,
            _ => return Err(PlugKitError::InvalidHandle),
        };
        let mmio_base = match device.property("pci.mmio_base")? {
            Some(DeviceProperty::U64(v)) => v,
            Some(DeviceProperty::U32(v)) => v as u64,
            _ => return Err(PlugKitError::InvalidHandle),
        };
        let mmio_size = match device.property("pci.mmio_size")? {
            Some(DeviceProperty::U64(v)) => v,
            Some(DeviceProperty::U32(v)) => v as u64,
            _ => return Err(PlugKitError::InvalidHandle),
        };
        let vendor = device.vendor_id().unwrap_or_default() as u16;
        let device_id = device.device_id().unwrap_or_default() as u16;
        enumerate_xhci_controller(
            PciLocation {
                bus,
                device: dev,
                function: func,
            },
            XhciBar {
                phys_base: mmio_base,
                size: mmio_size,
            },
            vendor,
            device_id,
        );
        Ok(())
    }

    fn stop(_device: PlugKitDevice) -> PlugKitResult<()> {
        Ok(())
    }
}

pub fn run() -> ! {
    platform::println!("usb-driver: start");
    pci_scan();
    platform::println!("usb-driver: enumeration complete");
    platform::process::exit(0)
}
