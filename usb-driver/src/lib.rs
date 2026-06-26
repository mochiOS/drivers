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
const XHCI_TRB_TYPE_ENABLE_SLOT: u32 = 9;
const XHCI_TRB_TYPE_ADDRESS_DEVICE: u32 = 11;
const XHCI_TRB_TYPE_SETUP_STAGE: u32 = 2;
const XHCI_TRB_TYPE_DATA_STAGE: u32 = 3;
const XHCI_TRB_TYPE_STATUS_STAGE: u32 = 4;
const XHCI_TRB_TYPE_TRANSFER_EVENT: u32 = 32;
const XHCI_TRB_TYPE_COMMAND_COMPLETION: u32 = 33;
const XHCI_TRB_CYCLE: u32 = 1 << 0;
const XHCI_TRB_TC: u32 = 1 << 1;
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
const XHCI_PORTSC_CCS: u32 = 1 << 0;
const XHCI_PORTSC_PED: u32 = 1 << 1;
const XHCI_PORTSC_OCA: u32 = 1 << 3;
const XHCI_PORTSC_PR: u32 = 1 << 4;
const XHCI_PORTSC_PP: u32 = 1 << 9;
const XHCI_PORTSC_SPEED_SHIFT: u32 = 10;
const XHCI_PORTSC_SPEED_MASK: u32 = 0xF << XHCI_PORTSC_SPEED_SHIFT;
const XHCI_PORTSC_CHANGE_BITS: u32 =
    (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20) | (1 << 21) | (1 << 22) | (1 << 23);

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
    virt: u64,
    phys: u64,
    len: usize,
}

struct CommandRingState {
    next_index: usize,
    cycle: u32,
}

#[repr(align(4096))]
struct Page4096([u8; 4096]);

static mut XHCI_DCBAA_PAGE: Page4096 = Page4096([0; 4096]);
static mut XHCI_COMMAND_RING_PAGE: Page4096 = Page4096([0; 4096]);
static mut XHCI_EVENT_RING_PAGE: Page4096 = Page4096([0; 4096]);
static mut XHCI_ERST_PAGE: Page4096 = Page4096([0; 4096]);
static mut XHCI_OUTPUT_CTX_PAGE: Page4096 = Page4096([0; 4096]);
static mut XHCI_INPUT_CTX_PAGE: Page4096 = Page4096([0; 4096]);
static mut XHCI_EP0_RING_PAGE: Page4096 = Page4096([0; 4096]);
static mut XHCI_DESCRIPTOR_PAGE: Page4096 = Page4096([0; 4096]);

impl DmaPage {
    fn from_static(page: *mut Page4096) -> Result<Self, syscall::SysError> {
        let virt = page.cast::<u8>() as u64;
        let phys = platform::memory::get_physical_addr(virt)?;
        Ok(Self {
            virt,
            phys,
            len: 4096,
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

    fn zero(&self) {
        // SAFETY: DMA page is mapped writable in this process.
        unsafe { core::ptr::write_bytes(self.ptr(), 0, self.len) }
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
    platform::println!("usb-driver: timeout waiting for {}", label);
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

fn init_command_and_event_rings(
    mmio: &MmioRegion,
    cap_length: usize,
    rtsoff: u32,
) -> Result<(DmaPage, DmaPage, DmaPage, DmaPage), syscall::SysError> {
    let dcbaa = {
        DmaPage::from_static(&raw mut XHCI_DCBAA_PAGE)?
    };
    let command_ring = {
        DmaPage::from_static(&raw mut XHCI_COMMAND_RING_PAGE)?
    };
    let event_ring = {
        DmaPage::from_static(&raw mut XHCI_EVENT_RING_PAGE)?
    };
    let erst = {
        DmaPage::from_static(&raw mut XHCI_ERST_PAGE)?
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

    let ir0 = rtsoff as usize + XHCI_RT_IR0;
    mmio.write_u32(ir0 + XHCI_IR_IMAN, 0);
    mmio.write_u32(ir0 + XHCI_IR_IMOD, 0);
    mmio.write_u32(ir0 + XHCI_IR_ERSTSZ, 1);
    mmio.write_u64(ir0 + XHCI_IR_ERSTBA, erst.phys);
    mmio.write_u64(ir0 + XHCI_IR_ERDP, event_ring.phys);

    Ok((dcbaa, command_ring, event_ring, erst))
}

fn ring_doorbell(mmio: &MmioRegion, dboff: u32, doorbell_index: u32, target: u32, stream_id: u16) {
    let offset = dboff as usize + (doorbell_index as usize) * 4;
    mmio.write_u32(offset, target | ((stream_id as u32) << 16));
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
    rtsoff: u32,
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
        let ir0 = rtsoff as usize + XHCI_RT_IR0;
        mmio.write_u64(ir0 + XHCI_IR_ERDP, event_ring.phys + next_offset as u64);
    }

    completion
}

fn wait_transfer_event(
    mmio: &MmioRegion,
    rtsoff: u32,
    event_ring: &DmaPage,
    slot_id: u8,
) -> Option<u32> {
    let trb_size = size_of::<Trb>();
    let event_count = event_ring.len / trb_size;
    wait_until("transfer event", 100_000, || {
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
        let event_slot_id = ((control >> 24) & 0xff) as u8;
        if trb_type == XHCI_TRB_TYPE_TRANSFER_EVENT && event_slot_id == slot_id {
            let status = event_ring.read_u32(offset + 8);
            completion = Some((status >> 24) & 0xff);
        }
        event_ring.write_u32(offset + 12, 0);
    }

    if consumed != 0 {
        let next_offset = if consumed >= event_count { 0 } else { consumed * trb_size };
        let ir0 = rtsoff as usize + XHCI_RT_IR0;
        mmio.write_u64(ir0 + XHCI_IR_ERDP, event_ring.phys + next_offset as u64);
    }

    completion
}

fn enable_slot(
    mmio: &MmioRegion,
    dboff: u32,
    rtsoff: u32,
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
    ring_doorbell(mmio, dboff, 0, 0, 0);
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

fn init_slot_contexts(
    dcbaa: &DmaPage,
    hccparams1: u32,
    slot_id: u8,
    root_port: u8,
    speed_id: u32,
) -> Result<(DmaPage, DmaPage, DmaPage, TransferRingState), syscall::SysError> {
    let output_ctx = {
        DmaPage::from_static(&raw mut XHCI_OUTPUT_CTX_PAGE)?
    };
    let input_ctx = {
        DmaPage::from_static(&raw mut XHCI_INPUT_CTX_PAGE)?
    };
    let ep0_ring = {
        DmaPage::from_static(&raw mut XHCI_EP0_RING_PAGE)?
    };
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

    dcbaa.write_u64(slot_id as usize * 8, output_ctx.phys);
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
    platform::println!(
        "usb-driver: slot {} contexts initialized ctx_size={} output=0x{:016x} input=0x{:016x} ep0_ring=0x{:016x} port={} speed={}",
        slot_id,
        context_size,
        output_ctx.phys,
        input_ctx.phys,
        ep0_ring.phys,
        root_port,
        speed_id
    );

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
    dboff: u32,
    rtsoff: u32,
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
    ring_doorbell(mmio, dboff, 0, 0, 0);
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

fn read_device_descriptor(
    mmio: &MmioRegion,
    dboff: u32,
    rtsoff: u32,
    event_ring: &DmaPage,
    slot_id: u8,
    ep0_ring: &DmaPage,
    ring_state: &mut TransferRingState,
) -> Option<(u16, u16)> {
    let descriptor = DmaPage::from_static(&raw mut XHCI_DESCRIPTOR_PAGE).ok()?;
    descriptor.zero();
    clear_event_trbs(event_ring);

    let setup_packet =
        0x0012_0000_0100_0680u64; // GET_DESCRIPTOR(Device, index=0, length=18), bmRequestType=0x80
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        setup_packet,
        8,
        (XHCI_TRB_TYPE_SETUP_STAGE << 10) | XHCI_TRB_IDT | (3 << 16),
    );
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        descriptor.phys,
        18,
        (XHCI_TRB_TYPE_DATA_STAGE << 10) | XHCI_TRB_DIR_IN,
    );
    queue_transfer_trb(
        ep0_ring,
        ring_state,
        0,
        0,
        (XHCI_TRB_TYPE_STATUS_STAGE << 10) | XHCI_TRB_IOC,
    );
    ring_doorbell(mmio, dboff, slot_id as u32, 1, 0);
    let completion = wait_transfer_event(mmio, rtsoff, event_ring, slot_id)?;
    if completion != XHCI_CC_SUCCESS {
        platform::println!(
            "usb-driver: get descriptor transfer failed completion={}",
            completion
        );
        return None;
    }

    let vendor = descriptor.read_u32(8) as u16;
    let product = (descriptor.read_u32(8) >> 16) as u16;
    Some((vendor, product))
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
    port_out(PCI_CFG_ADDR, addr as u64, 4).ok()?;
    let value = port_in(PCI_CFG_DATA, 4).ok()?;
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
        let value = pci_read_u32(loc, offset).unwrap_or(0);
        platform::println!(
            "usb-driver: bar{} raw=0x{:08x}",
            bar_idx,
            value
        );
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
    let dboff = mmio.read_u32(XHCI_CAP_DBOFF) & !0x3;
    let rtsoff = mmio.read_u32(XHCI_CAP_RTSOFF) & !0x1f;
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
    platform::println!(
        "usb-driver: xhci usbcmd=0x{:08x} usbsts=0x{:08x} pagesize=0x{:08x} config=0x{:08x}",
        usbcmd,
        usbsts,
        pagesize,
        config
    );

    if !xhci_stop_and_reset(&mmio, cap_length) {
        platform::println!("usb-driver: xhci reset failed");
        return;
    }
    let Ok((dcbaa, command_ring, event_ring, _erst)) =
        init_command_and_event_rings(&mmio, cap_length, rtsoff)
    else {
        platform::println!("usb-driver: xhci ring initialization failed");
        return;
    };
    let mut command_ring_state = CommandRingState {
        next_index: 0,
        cycle: XHCI_TRB_CYCLE,
    };
    platform::println!(
        "usb-driver: command ring=0x{:016x} event ring=0x{:016x} dcbaa=0x{:016x}",
        command_ring.phys,
        event_ring.phys,
        dcbaa.phys
    );
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
        platform::println!(
            "usb-driver: port{} connected={} enabled={} power={} reset={} over_current={} speed={} status=0x{:08x}",
            port_index + 1,
            connected as u8,
            enabled as u8,
            power as u8,
            reset as u8,
            over_current as u8,
            xhci_port_speed_name(speed_id),
            portsc
        );
    }

    if let Some((root_port, speed_id)) = selected_port {
        if let Some(slot_id) = enable_slot(
            &mmio,
            dboff,
            rtsoff,
            &command_ring,
            &mut command_ring_state,
            &event_ring,
        ) {
            platform::println!("usb-driver: enable slot ok slot_id={}", slot_id);
            match init_slot_contexts(&dcbaa, hccparams1, slot_id, root_port, speed_id) {
                Ok((_output_ctx, input_ctx, ep0_ring, mut ep0_ring_state)) => {
                    if address_device(
                        &mmio,
                        dboff,
                        rtsoff,
                        &command_ring,
                        &mut command_ring_state,
                        &event_ring,
                        &input_ctx,
                        slot_id,
                    ) {
                        platform::println!(
                            "usb-driver: address device ok slot_id={} root_port={}",
                            slot_id,
                            root_port
                        );
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
                        } else {
                            platform::println!("usb-driver: device descriptor read failed");
                        }
                    } else {
                        platform::println!("usb-driver: address device timed out or failed");
                    }
                }
                Err(_) => {
                    platform::println!("usb-driver: slot context initialization failed");
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

                platform::println!(
                    "usb-driver: candidate bus={:02x} dev={:02x} func={} vendor=0x{:04x} device=0x{:04x} class=0x{:02x} subclass=0x{:02x} prog_if=0x{:02x}",
                    bus,
                    device,
                    function,
                    vendor,
                    device_id,
                    class,
                    subclass,
                    prog_if
                );
                log_pci_bars(loc);

                let Some(bar) = find_xhci_bar(loc) else {
                    platform::println!(
                        "usb-driver: candidate bus={:02x} dev={:02x} func={} has no MMIO BAR",
                        bus,
                        device,
                        function
                    );
                    continue;
                };

                pci_command_enable_memory_and_bus_master(loc);
                platform::println!(
                    "usb-driver: candidate bus={:02x} dev={:02x} func={} mmio_base=0x{:016x} mmio_size=0x{:x}",
                    bus,
                    device,
                    function,
                    bar.phys_base,
                    bar.size
                );
                let Ok(mmio_probe) = MmioRegion::map(bar.phys_base, core::cmp::min(bar.size, 0x1000))
                else {
                    platform::println!(
                        "usb-driver: candidate bus={:02x} dev={:02x} func={} mmio map failed base=0x{:016x} size=0x{:x}",
                        bus,
                        device,
                        function,
                        bar.phys_base,
                        bar.size
                    );
                    continue;
                };
                if !looks_like_xhci(&mmio_probe) {
                    platform::println!(
                        "usb-driver: candidate bus={:02x} dev={:02x} func={} not xhci caplen=0x{:02x} hci=0x{:04x} hcs1=0x{:08x}",
                        bus,
                        device,
                        function,
                        mmio_probe.read_u8(XHCI_CAP_CAPLENGTH),
                        mmio_probe.read_u16(XHCI_CAP_HCIVERSION),
                        mmio_probe.read_u32(XHCI_CAP_HCSPARAMS1)
                    );
                    continue;
                }

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
