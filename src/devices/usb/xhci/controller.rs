use alloc::{boxed::Box, collections::BTreeMap, sync::Arc, vec, vec::Vec};
use core::ptr;

use crate::{
    arch::apic,
    devices::{
        discovery::pcie::{
            PCI_CAP_MSI, PcieFunction, enable_bus_mastering, enable_mem_space, find_capability,
            get_msix_table, map_bar, register_msi_handler, register_msix_handler,
        },
        usb::xhci::{
            context::{
                CTX_SIZE_32, CTX_SIZE_64, ContextBlob, Dcbaa, endpoint_context_dwords,
                ep0_context_dwords, slot_context_dw0, slot_context_dw1,
            },
            descriptors::{
                DeviceDescriptor, EndpointDescriptor, ParsedConfig, parse_configuration,
            },
            device::UsbDevice,
            events::EventRingHandler,
            registers::{
                BMREQ_DEVICE_TO_HOST, BMREQ_HOST_TO_DEVICE, CAP_DBOFF, CAP_HCCPARAMS1,
                CAP_HCSPARAMS1, CAP_HCSPARAMS2, CAP_RTSOFF, CAP_VERSION_LENGTH,
                COMPLETION_SHORT_PACKET, COMPLETION_SUCCESS, CRCR_RCS, IMAN_IE, IMAN_IP,
                IR_ERDP_HI, IR_ERDP_LO, IR_ERSTBA_HI, IR_ERSTBA_LO, IR_ERSTSZ, IR_IMAN, IR_IMOD,
                OP_CONFIG, OP_CRCR_HI, OP_CRCR_LO, OP_DCBAAP_HI, OP_DCBAAP_LO, OP_PORT_REG_STRIDE,
                OP_PORT_REGS_BASE, OP_USBCMD, OP_USBSTS, PORT_SPEED_FULL, PORT_SPEED_HIGH,
                PORT_SPEED_LOW, PORT_SPEED_SUPER, PORTSC_CCS, PORTSC_PR, PORTSC_PRC,
                PORTSC_PRESERVE_MASK, PORTSC_SPEED_MASK, PORTSC_SPEED_SHIFT, RT_IR0_BASE,
                TRB_TRT_IN, TRB_TRT_NO_DATA, USB_DESC_CONFIGURATION, USB_DESC_DEVICE,
                USB_REQ_GET_DESCRIPTOR, USB_REQ_SET_CONFIGURATION, USBCMD_HCRST, USBCMD_INTE,
                USBCMD_RUN, USBLEGCTLSTS_DISABLE_SMI_AND_CLEAR, USBLEGCTLSTS_OFFSET,
                USBLEGSUP_BIOS_OWNED, USBLEGSUP_OS_OWNED, USBSTS_CNR, USBSTS_HCH,
                XECP_ID_USB_LEGACY_SUPPORT, event_completion_code, trb_get_slot_id,
            },
            ring::{
                EventRingState, ProducerRing, TRBS_PER_RING, Trb, build_address_device,
                build_configure_endpoint, build_data_stage, build_enable_slot, build_setup_stage,
                build_status_stage,
            },
        },
    },
    memory::dma::{DmaRegion, MmioRegion},
    print::kprintln,
    sync::{IntMutex, MutexLike, Promise},
    thread::spawn_thread,
};

const HANDOFF_SPIN_LIMIT: usize = 1_000_000;
const PORT_RESET_SPIN_LIMIT: usize = 1_000_000;
const CONTROLLER_READY_SPIN_LIMIT: usize = 10_000_000;
const XECP_MAX_HOPS: usize = 64;

/// Fields of a USB control-transfer SETUP packet (USB  9.3).
#[derive(Clone, Copy)]
struct ControlSetup {
    bm_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
}

pub struct CapParams {
    pub cap_length: u8,
    pub hci_version: u16,
    pub max_slots: u8,
    pub max_intrs: u16,
    pub max_ports: u8,
    pub max_scratchpad_buffers: u16,
    pub erst_max_log2: u8,
    pub context_size_64: bool,
    pub xecp_offset: usize,
    pub doorbell_offset: usize,
    pub runtime_offset: usize,
}

pub struct XhciController {
    pub pcie_func: PcieFunction,
    pub cap: CapParams,
    pub mmio: Arc<MmioRegion>,
    pub event_handler: Arc<EventRingHandler>,
    pub command_ring: IntMutex<ProducerRing>,

    // These DMA buffers must outlive the controller — the HC keeps reading
    // them via the addresses we programmed into DCBAAP, CRCR, ERSTBA.
    pub command_ring_dma: DmaRegion,
    pub event_ring_dma: DmaRegion,
    pub erst: DmaRegion,
    pub dcbaa: Dcbaa,
    pub scratchpad_array: Option<DmaRegion>,
    pub scratchpad_pages: Vec<DmaRegion>,

    pub devices: IntMutex<Vec<UsbDevice>>,
}

impl XhciController {
    /// Discovery runs with CPU IF=0, so this stays sync — no `Promise::get()`
    /// from the BSP's boot context (suspending it would corrupt the thread
    /// queue). Enumeration is deferred to `spawn_thread` at the end.
    pub fn bringup(mut pcie_func: PcieFunction) -> Option<Arc<XhciController>> {
        enable_mem_space(&mut pcie_func)?;
        enable_bus_mastering(&mut pcie_func)?;
        let mmio = Arc::new(map_bar(&mut pcie_func, 0)?);

        // SAFETY: BAR0 just mapped; all offsets we touch are inside it.
        let cap = unsafe { read_cap_params(&mmio) };
        log_caps(&cap);

        if cap.xecp_offset != 0 {
            // SAFETY: xECP entries are 32-bit aligned within BAR0.
            unsafe { walk_extended_capabilities(&mmio, cap.xecp_offset) };
        }

        let op_base = cap.cap_length as usize;

        halt_controller(&mmio, op_base)?;
        reset_controller(&mmio, op_base)?;

        unsafe {
            mmio.write::<u32>(op_base + OP_CONFIG, cap.max_slots as u32);
        }

        let dcbaa = Dcbaa::new();
        let (scratchpad_array, scratchpad_pages) = alloc_scratchpad(&dcbaa, &cap);

        let dcbaa_phys = dcbaa.phys_addr();
        unsafe {
            mmio.write::<u32>(op_base + OP_DCBAAP_LO, (dcbaa_phys & 0xFFFF_FFFF) as u32);
            mmio.write::<u32>(op_base + OP_DCBAAP_HI, (dcbaa_phys >> 32) as u32);
        }

        let command_ring_dma = DmaRegion::new(1);
        let command_ring = ProducerRing::new(&command_ring_dma);
        let cr_phys = command_ring.phys_addr as u64;
        unsafe {
            mmio.write::<u32>(
                op_base + OP_CRCR_LO,
                ((cr_phys & 0xFFFF_FFC0) | CRCR_RCS) as u32,
            );
            mmio.write::<u32>(op_base + OP_CRCR_HI, (cr_phys >> 32) as u32);
        }

        let event_ring_dma = DmaRegion::new(1);
        let erst = DmaRegion::new(1);
        // SAFETY: ERST DmaRegion is 4 KB, only first 16 B written.
        unsafe {
            let erst_p = erst.virt_addr() as *mut u32;
            ptr::write_volatile(
                erst_p.add(0),
                (event_ring_dma.phys_addr() & 0xFFFF_FFFF) as u32,
            );
            ptr::write_volatile(erst_p.add(1), (event_ring_dma.phys_addr() >> 32) as u32);
            ptr::write_volatile(erst_p.add(2), TRBS_PER_RING as u32);
            ptr::write_volatile(erst_p.add(3), 0);
        }
        let event_ring_state = EventRingState::new(&event_ring_dma);

        // ERSTBA must be programmed last — writing it commits the interrupter
        // config; if the HC starts using ERSTBA before ERSTSZ/ERDP are set
        // we'll get garbage events.
        let ir0 = cap.runtime_offset + RT_IR0_BASE;
        let erdp = event_ring_dma.phys_addr() as u64;
        unsafe {
            mmio.write::<u32>(ir0 + IR_ERSTSZ, 1u32);
            mmio.write::<u32>(ir0 + IR_ERDP_LO, (erdp & 0xFFFF_FFFF) as u32);
            mmio.write::<u32>(ir0 + IR_ERDP_HI, (erdp >> 32) as u32);
            mmio.write::<u32>(ir0 + IR_IMOD, 0);
            mmio.write::<u32>(ir0 + IR_IMAN, IMAN_IE | IMAN_IP);
            mmio.write::<u32>(ir0 + IR_ERSTBA_LO, (erst.phys_addr() & 0xFFFF_FFFF) as u32);
            mmio.write::<u32>(ir0 + IR_ERSTBA_HI, (erst.phys_addr() >> 32) as u32);
        }

        let event_handler = Arc::new(EventRingHandler {
            mmio: mmio.clone(),
            runtime_offset: cap.runtime_offset,
            state: IntMutex::new(event_ring_state),
            pending: IntMutex::new(BTreeMap::new()),
        });

        let handler_clone = event_handler.clone();
        let handler_box: Box<dyn (Fn() -> Option<()>) + Send + Sync> = Box::new(move || {
            apic::eoi();
            handler_clone.handle();
            Some(())
        });

        // Prefer MSI-X when present; some Intel xHCI controllers 
        // only expose legacy MSI, so fall back to that.
        match get_msix_table(&mut pcie_func) {
            Some((table_bir, msix_cap_off, table_off)) => {
                if table_bir != 0 {
                    kprintln!("[xhci] MSI-X table in BAR{} — unsupported", table_bir);
                    return None;
                }
                if register_msix_handler(
                    &mut pcie_func,
                    &mmio,
                    table_off,
                    msix_cap_off,
                    None,
                    handler_box,
                )
                .is_none()
                {
                    kprintln!("[xhci] register_msix_handler failed");
                    return None;
                }
                kprintln!("[xhci] using MSI-X");
            }
            None => match find_capability(&pcie_func, PCI_CAP_MSI) {
                Some(msi_cap_off) => {
                    if register_msi_handler(&mut pcie_func, msi_cap_off, None, handler_box)
                        .is_none()
                    {
                        kprintln!("[xhci] MSI registration failed");
                        return None;
                    }
                    kprintln!("[xhci] using MSI (cap @ {:#x})", msi_cap_off);
                }
                None => {
                    kprintln!("[xhci] no MSI or MSI-X capability — cannot deliver IRQs");
                    return None;
                }
            },
        }

        // USBCMD: enable interrupts, then RUN.
        unsafe {
            let cmd: u32 = mmio.read(op_base + OP_USBCMD);
            mmio.write::<u32>(op_base + OP_USBCMD, cmd | USBCMD_INTE | USBCMD_RUN);
        }
        // Wait for HCH=0 (controller running).
        let mut spins = 0;
        loop {
            let sts: u32 = unsafe { mmio.read(op_base + OP_USBSTS) };
            if (sts & USBSTS_HCH) == 0 {
                break;
            }
            spins += 1;
            if spins >= CONTROLLER_READY_SPIN_LIMIT {
                kprintln!("[xhci] controller failed to leave halted state");
                return None;
            }
            core::hint::spin_loop();
        }
        kprintln!("[xhci] controller running");

        let controller = Arc::new(XhciController {
            pcie_func,
            cap,
            mmio,
            event_handler,
            command_ring: IntMutex::new(command_ring),
            command_ring_dma,
            event_ring_dma,
            erst,
            dcbaa,
            scratchpad_array,
            scratchpad_pages,
            devices: IntMutex::new(vec![]),
        });

        // Issuing commands needs `Promise::get()`, which requires a real
        // thread context with IRQs enabled — neither holds in the discovery
        // thread, so enumeration runs on a worker.
        let enum_clone = controller.clone();
        spawn_thread(move || {
            enum_clone.enumerate_ports();
        });

        Some(controller)
    }

    fn ring_doorbell(&self, idx: usize, target: u32) {
        let off = self.cap.doorbell_offset + idx * 4;
        // SAFETY: idx is bounded by max_slots+1 ≤ 257; doorbell array fits in BAR0.
        unsafe { self.mmio.write::<u32>(off, target) };
    }

    fn portsc_off(&self, port: u8) -> usize {
        self.cap.cap_length as usize + OP_PORT_REGS_BASE + (port as usize - 1) * OP_PORT_REG_STRIDE
    }

    fn issue_command(&self, dw0: u32, dw1: u32, dw2: u32, dw3: u32) -> Trb {
        let promise = Arc::new(Promise::new());
        let trb_phys = {
            let mut ring = self.command_ring.lock();
            ring.enqueue(dw0, dw1, dw2, dw3)
        };
        self.event_handler
            .pending
            .lock()
            .insert(trb_phys, promise.clone());
        self.ring_doorbell(0, 0);
        promise.get()
    }

    fn issue_control_in(
        &self,
        slot_id: u8,
        ep0_tr: &IntMutex<ProducerRing>,
        setup: ControlSetup,
        length: u16,
    ) -> Option<(Vec<u8>, Trb)> {
        let buffer_dma = if length > 0 {
            Some(DmaRegion::new_bytes(length as usize))
        } else {
            None
        };

        let promise = Arc::new(Promise::new());
        let status_phys = {
            let mut ring = ep0_tr.lock();
            let trt = if length > 0 {
                TRB_TRT_IN
            } else {
                TRB_TRT_NO_DATA
            };
            let (s0, s1, s2, s3) = build_setup_stage(
                setup.bm_request_type,
                setup.b_request,
                setup.w_value,
                setup.w_index,
                length,
                trt,
            );
            ring.enqueue(s0, s1, s2, s3);

            if let Some(buf) = &buffer_dma {
                let (d0, d1, d2, d3) =
                    build_data_stage(buf.phys_addr() as u64, length as u32, true);
                ring.enqueue(d0, d1, d2, d3);
            }

            // Status Stage direction is opposite to the data stage (or IN if
            // no data); IOC=1 so we get a completion event for the transfer.
            let (st0, st1, st2, st3) = build_status_stage(length == 0, true);
            ring.enqueue(st0, st1, st2, st3)
        };

        self.event_handler
            .pending
            .lock()
            .insert(status_phys, promise.clone());
        self.ring_doorbell(slot_id as usize, 1);

        let event = promise.get();
        let data = match buffer_dma {
            Some(buf) => buf.as_slice()[..length as usize].to_vec(),
            None => vec![],
        };
        Some((data, event))
    }

    fn issue_control_out_no_data(
        &self,
        slot_id: u8,
        ep0_tr: &IntMutex<ProducerRing>,
        setup: ControlSetup,
    ) -> Trb {
        let promise = Arc::new(Promise::new());
        let status_phys = {
            let mut ring = ep0_tr.lock();
            let (s0, s1, s2, s3) = build_setup_stage(
                setup.bm_request_type,
                setup.b_request,
                setup.w_value,
                setup.w_index,
                0,
                TRB_TRT_NO_DATA,
            );
            ring.enqueue(s0, s1, s2, s3);
            let (st0, st1, st2, st3) = build_status_stage(true, true);
            ring.enqueue(st0, st1, st2, st3)
        };

        self.event_handler
            .pending
            .lock()
            .insert(status_phys, promise.clone());
        self.ring_doorbell(slot_id as usize, 1);
        promise.get()
    }

    fn enumerate_ports(&self) {
        for port in 1..=self.cap.max_ports {
            self.enumerate_port(port);
        }
    }

    fn enumerate_port(&self, port: u8) {
        let portsc_off = self.portsc_off(port);
        let portsc: u32 = unsafe { self.mmio.read(portsc_off) };
        if (portsc & PORTSC_CCS) == 0 {
            return;
        }
        kprintln!("[xhci] port {} connected (PORTSC={:#x})", port, portsc);

        let preserved = portsc & PORTSC_PRESERVE_MASK;
        unsafe {
            self.mmio.write::<u32>(portsc_off, preserved | PORTSC_PR);
        }

        let mut spins = 0;
        loop {
            let sc: u32 = unsafe { self.mmio.read(portsc_off) };
            if (sc & PORTSC_PRC) != 0 {
                break;
            }
            spins += 1;
            if spins >= PORT_RESET_SPIN_LIMIT {
                kprintln!("[xhci] port {} reset timed out", port);
                return;
            }
            core::hint::spin_loop();
        }

        let portsc: u32 = unsafe { self.mmio.read(portsc_off) };
        let preserved = portsc & PORTSC_PRESERVE_MASK;
        unsafe {
            self.mmio.write::<u32>(portsc_off, preserved | PORTSC_PRC);
        }
        let speed = (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT;
        kprintln!("[xhci] port {} reset complete, speed={}", port, speed);

        let (e0, e1, e2, e3) = build_enable_slot();
        let event = self.issue_command(e0, e1, e2, e3);
        let cc = event_completion_code(event.data[2]);
        if cc != COMPLETION_SUCCESS {
            kprintln!("[xhci] Enable Slot failed cc={}", cc);
            return;
        }
        let slot_id = trb_get_slot_id(event.data[3]);
        kprintln!("[xhci] port {} → slot {}", port, slot_id);

        let ctx_size = if self.cap.context_size_64 {
            CTX_SIZE_64
        } else {
            CTX_SIZE_32
        };
        let device_ctx = ContextBlob::new(33, ctx_size);
        let input_ctx = ContextBlob::new(34, ctx_size);
        let ep0_tr_dma = DmaRegion::new(1);
        let ep0_tr_initial = ProducerRing::new(&ep0_tr_dma);

        let initial_mps0 = match speed {
            PORT_SPEED_LOW | PORT_SPEED_FULL => 8u32,
            PORT_SPEED_HIGH => 64,
            PORT_SPEED_SUPER => 512,
            _ => 8,
        };

        // Input Context for the first AddressDevice: drop=0, add A0|A1
        // (slot + EP0). Slot context_entries=1 because only EP0 exists yet.
        input_ctx.write_dword(0, 0, 0);
        input_ctx.write_dword(0, 1, 0x03);
        input_ctx.write_dword(1, 0, slot_context_dw0(0, speed, 1));
        input_ctx.write_dword(1, 1, slot_context_dw1(port));
        let ep0_phys = ep0_tr_dma.phys_addr() as u64;
        for (i, &dw) in ep0_context_dwords(initial_mps0, ep0_phys, true)
            .iter()
            .enumerate()
        {
            input_ctx.write_dword(2, i, dw);
        }

        self.dcbaa.set(slot_id, device_ctx.phys_addr());

        // FS/LS devices report their actual MaxPacketSize0 only after a
        // partial GET_DESCRIPTOR; we Address with BSR=1 first so the HC sets
        // the slot up but doesn't issue SET_ADDRESS over the wire yet.
        let needs_mps_dance = speed == PORT_SPEED_FULL || speed == PORT_SPEED_LOW;

        let (a0, a1, a2, a3) =
            build_address_device(input_ctx.phys_addr(), slot_id, needs_mps_dance);
        let event = self.issue_command(a0, a1, a2, a3);
        let cc = event_completion_code(event.data[2]);
        if cc != COMPLETION_SUCCESS {
            kprintln!(
                "[xhci] AddressDevice (BSR={}) failed cc={}",
                needs_mps_dance,
                cc
            );
            return;
        }

        let ep0_tr = IntMutex::new(ep0_tr_initial);

        if needs_mps_dance {
            let Some((data, ev)) = self.issue_control_in(
                slot_id,
                &ep0_tr,
                ControlSetup {
                    bm_request_type: BMREQ_DEVICE_TO_HOST,
                    b_request: USB_REQ_GET_DESCRIPTOR,
                    w_value: (USB_DESC_DEVICE as u16) << 8,
                    w_index: 0,
                },
                8,
            ) else {
                return;
            };
            let cc = event_completion_code(ev.data[2]);
            if cc != COMPLETION_SUCCESS && cc != COMPLETION_SHORT_PACKET {
                kprintln!("[xhci] FS/LS GET_DESCRIPTOR(8) failed cc={}", cc);
                return;
            }
            let actual_mps0 = data[7] as u32;
            kprintln!("[xhci] slot {} FS/LS MPS0 fix → {}", slot_id, actual_mps0);

            // EP context DW1 bits[31:16] hold MaxPacketSize.
            let mut ep0_dw1 = input_ctx.read_dword(2, 1);
            ep0_dw1 = (ep0_dw1 & 0x0000_FFFF) | (actual_mps0 << 16);
            input_ctx.write_dword(2, 1, ep0_dw1);

            let (a0, a1, a2, a3) = build_address_device(input_ctx.phys_addr(), slot_id, false);
            let event = self.issue_command(a0, a1, a2, a3);
            let cc = event_completion_code(event.data[2]);
            if cc != COMPLETION_SUCCESS {
                kprintln!("[xhci] AddressDevice (BSR=0) failed cc={}", cc);
                return;
            }
        }

        let Some((data, ev)) = self.issue_control_in(
            slot_id,
            &ep0_tr,
            ControlSetup {
                bm_request_type: BMREQ_DEVICE_TO_HOST,
                b_request: USB_REQ_GET_DESCRIPTOR,
                w_value: (USB_DESC_DEVICE as u16) << 8,
                w_index: 0,
            },
            18,
        ) else {
            return;
        };
        let cc = event_completion_code(ev.data[2]);
        if cc != COMPLETION_SUCCESS && cc != COMPLETION_SHORT_PACKET {
            kprintln!("[xhci] GET_DESCRIPTOR(Device,18) failed cc={}", cc);
            return;
        }
        let device_desc = DeviceDescriptor::from_bytes(&data);
        if let Some(d) = &device_desc {
            kprintln!(
                "[xhci] slot {} VID:PID {:04x}:{:04x} class={:02x}:{:02x}:{:02x} mps0={} cfgs={}",
                slot_id,
                d.id_vendor,
                d.id_product,
                d.b_device_class,
                d.b_device_subclass,
                d.b_device_protocol,
                d.b_max_packet_size0,
                d.b_num_configurations
            );
        }

        // Two-step config descriptor read: first 9 bytes give wTotalLength,
        // second pass pulls the whole blob.
        let config_setup = ControlSetup {
            bm_request_type: BMREQ_DEVICE_TO_HOST,
            b_request: USB_REQ_GET_DESCRIPTOR,
            w_value: (USB_DESC_CONFIGURATION as u16) << 8,
            w_index: 0,
        };
        let Some((data, _)) = self.issue_control_in(slot_id, &ep0_tr, config_setup, 9) else {
            return;
        };
        if data.len() < 9 {
            kprintln!(
                "[xhci] GET_DESCRIPTOR(Config,9) returned {} bytes",
                data.len()
            );
            return;
        }
        let total_len = u16::from_le_bytes([data[2], data[3]]);

        let Some((data, _)) = self.issue_control_in(slot_id, &ep0_tr, config_setup, total_len)
        else {
            return;
        };
        let parsed = parse_configuration(&data);
        if let Some(p) = &parsed {
            kprintln!(
                "[xhci] slot {} cfg={} ifaces={}",
                slot_id,
                p.config.b_configuration_value,
                p.interfaces.len()
            );
            for (iface, eps) in &p.interfaces {
                kprintln!(
                    "[xhci]   iface {}.{} class={:02x}:{:02x}:{:02x} eps={}",
                    iface.b_interface_number,
                    iface.b_alternate_setting,
                    iface.b_interface_class,
                    iface.b_interface_subclass,
                    iface.b_interface_protocol,
                    eps.len()
                );
                for ep in eps {
                    kprintln!(
                        "[xhci]     EP {} {} type={} mps={} interval={}",
                        ep.ep_number(),
                        if ep.is_in() { "IN" } else { "OUT" },
                        ep.transfer_type(),
                        ep.w_max_packet_size,
                        ep.b_interval
                    );
                }
            }
        }

        // SET_CONFIGURATION (host-to-device, no data).
        if let Some(p) = &parsed {
            let event = self.issue_control_out_no_data(
                slot_id,
                &ep0_tr,
                ControlSetup {
                    bm_request_type: BMREQ_HOST_TO_DEVICE,
                    b_request: USB_REQ_SET_CONFIGURATION,
                    w_value: p.config.b_configuration_value as u16,
                    w_index: 0,
                },
            );
            let cc = event_completion_code(event.data[2]);
            if cc != COMPLETION_SUCCESS {
                kprintln!("[xhci] SET_CONFIGURATION failed cc={}", cc);
                return;
            }
            kprintln!("[xhci] slot {} SET_CONFIGURATION ok", slot_id);
        }

        // Rebuild the input context with full endpoint set, then issue
        // Configure Endpoint. The new transfer rings are allocated but
        // unused in Stage 1 — class drivers will hook them up later.
        let current_mps0 = ((input_ctx.read_dword(2, 1)) >> 16) & 0xFFFF;
        if let Some(p) = &parsed {
            let new_rings =
                configure_endpoint_input_ctx(&input_ctx, p, speed, port, ep0_phys, current_mps0);

            let (c0, c1, c2, c3) = build_configure_endpoint(input_ctx.phys_addr(), slot_id);
            let event = self.issue_command(c0, c1, c2, c3);
            let cc = event_completion_code(event.data[2]);
            if cc != COMPLETION_SUCCESS {
                kprintln!("[xhci] Configure Endpoint failed cc={}", cc);
                return;
            }
            kprintln!(
                "[xhci] slot {} Configure Endpoint ok ({} non-control EPs)",
                slot_id,
                new_rings.len()
            );
            let _ = new_rings;
        }

        let dev = UsbDevice {
            slot_id,
            port,
            speed,
            address: 0,
            device_descriptor: device_desc,
            config: parsed,
            input_ctx,
            device_ctx,
            ep0_tr_dma,
            ep0_tr,
        };
        self.devices.lock().push(dev);
    }
}

unsafe fn read_cap_params(mmio: &MmioRegion) -> CapParams {
    let version_length: u32 = unsafe { mmio.read(CAP_VERSION_LENGTH) };
    let cap_length = (version_length & 0xFF) as u8;
    let hci_version = ((version_length >> 16) & 0xFFFF) as u16;

    let hcsparams1: u32 = unsafe { mmio.read(CAP_HCSPARAMS1) };
    let max_slots = (hcsparams1 & 0xFF) as u8;
    let max_intrs = ((hcsparams1 >> 8) & 0x7FF) as u16;
    let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;

    let hcsparams2: u32 = unsafe { mmio.read(CAP_HCSPARAMS2) };
    let erst_max_log2 = ((hcsparams2 >> 4) & 0xF) as u8;
    let scratch_hi = (hcsparams2 >> 21) & 0x1F;
    let scratch_lo = (hcsparams2 >> 27) & 0x1F;
    let max_scratchpad_buffers = ((scratch_hi << 5) | scratch_lo) as u16;

    let hccparams1: u32 = unsafe { mmio.read(CAP_HCCPARAMS1) };
    let context_size_64 = (hccparams1 & (1 << 2)) != 0;
    let xecp_offset = (((hccparams1 >> 16) & 0xFFFF) as usize) * 4;

    let dboff: u32 = unsafe { mmio.read(CAP_DBOFF) };
    let rtsoff: u32 = unsafe { mmio.read(CAP_RTSOFF) };
    let doorbell_offset = (dboff & !0x3) as usize;
    let runtime_offset = (rtsoff & !0x1F) as usize;

    CapParams {
        cap_length,
        hci_version,
        max_slots,
        max_intrs,
        max_ports,
        max_scratchpad_buffers,
        erst_max_log2,
        context_size_64,
        xecp_offset,
        doorbell_offset,
        runtime_offset,
    }
}

fn log_caps(cap: &CapParams) {
    kprintln!(
        "[xhci]   HCIVERSION={:x}.{:02x} CAPLENGTH={:#x}",
        (cap.hci_version >> 8) & 0xFF,
        cap.hci_version & 0xFF,
        cap.cap_length
    );
    kprintln!(
        "[xhci]   slots={} intrs={} ports={} scratchpad={} ctx={}B erst_max=2^{}",
        cap.max_slots,
        cap.max_intrs,
        cap.max_ports,
        cap.max_scratchpad_buffers,
        if cap.context_size_64 { 64 } else { 32 },
        cap.erst_max_log2,
    );
    kprintln!(
        "[xhci]   xECP={:#x} dboff={:#x} rtsoff={:#x}",
        cap.xecp_offset,
        cap.doorbell_offset,
        cap.runtime_offset
    );
}

unsafe fn walk_extended_capabilities(mmio: &MmioRegion, first_offset: usize) {
    let mut offset = first_offset;
    let mut hops = 0;
    loop {
        if hops >= XECP_MAX_HOPS {
            kprintln!("[xhci]   xECP walk aborted: too many hops");
            return;
        }
        let entry: u32 = unsafe { mmio.read(offset) };
        let cap_id = (entry & 0xFF) as u8;
        let next_dwords = ((entry >> 8) & 0xFF) as usize;
        if cap_id == XECP_ID_USB_LEGACY_SUPPORT {
            unsafe { perform_legacy_handoff(mmio, offset, entry) };
        }
        if next_dwords == 0 {
            return;
        }
        offset += next_dwords * 4;
        hops += 1;
    }
}

unsafe fn perform_legacy_handoff(mmio: &MmioRegion, offset: usize, current_entry: u32) {
    if (current_entry & USBLEGSUP_BIOS_OWNED) == 0 {
        kprintln!("[xhci]   legacy support: BIOS does not own controller");
    } else {
        kprintln!("[xhci]   legacy support: requesting handoff from BIOS");
        unsafe {
            mmio.write::<u32>(offset, current_entry | USBLEGSUP_OS_OWNED);
        }
        let mut spins = 0;
        loop {
            let live: u32 = unsafe { mmio.read(offset) };
            if (live & USBLEGSUP_BIOS_OWNED) == 0 && (live & USBLEGSUP_OS_OWNED) != 0 {
                kprintln!("[xhci]   legacy support: handoff complete");
                break;
            }
            spins += 1;
            if spins >= HANDOFF_SPIN_LIMIT {
                kprintln!("[xhci]   legacy support: handoff timed out, forcing");
                let forced = (live | USBLEGSUP_OS_OWNED) & !USBLEGSUP_BIOS_OWNED;
                unsafe { mmio.write::<u32>(offset, forced) };
                break;
            }
            core::hint::spin_loop();
        }
    }
    unsafe {
        mmio.write::<u32>(
            offset + USBLEGCTLSTS_OFFSET,
            USBLEGCTLSTS_DISABLE_SMI_AND_CLEAR,
        );
    }
}

fn halt_controller(mmio: &MmioRegion, op_base: usize) -> Option<()> {
    unsafe {
        let cmd: u32 = mmio.read(op_base + OP_USBCMD);
        mmio.write::<u32>(op_base + OP_USBCMD, cmd & !USBCMD_RUN);
    }
    let mut spins = 0;
    loop {
        let sts: u32 = unsafe { mmio.read(op_base + OP_USBSTS) };
        if (sts & USBSTS_HCH) != 0 {
            return Some(());
        }
        spins += 1;
        if spins >= CONTROLLER_READY_SPIN_LIMIT {
            kprintln!("[xhci] halt timed out");
            return None;
        }
        core::hint::spin_loop();
    }
}

fn reset_controller(mmio: &MmioRegion, op_base: usize) -> Option<()> {
    unsafe {
        let cmd: u32 = mmio.read(op_base + OP_USBCMD);
        mmio.write::<u32>(op_base + OP_USBCMD, cmd | USBCMD_HCRST);
    }
    let mut spins = 0;
    loop {
        let cmd: u32 = unsafe { mmio.read(op_base + OP_USBCMD) };
        let sts: u32 = unsafe { mmio.read(op_base + OP_USBSTS) };
        if (cmd & USBCMD_HCRST) == 0 && (sts & USBSTS_CNR) == 0 {
            return Some(());
        }
        spins += 1;
        if spins >= CONTROLLER_READY_SPIN_LIMIT {
            kprintln!("[xhci] reset timed out");
            return None;
        }
        core::hint::spin_loop();
    }
}

fn alloc_scratchpad(dcbaa: &Dcbaa, cap: &CapParams) -> (Option<DmaRegion>, Vec<DmaRegion>) {
    if cap.max_scratchpad_buffers == 0 {
        dcbaa.set(0, 0);
        return (None, vec![]);
    }
    let scratch_array_dma = DmaRegion::new(1);
    let mut pages = vec![];
    for i in 0..cap.max_scratchpad_buffers as usize {
        let page = DmaRegion::new(1);
        // SAFETY: scratch_array_dma is 4 KB, holds up to 512 8-byte pointers.
        let p = (scratch_array_dma.virt_addr() + i * 8) as *mut u64;
        unsafe {
            ptr::write_volatile(p, page.phys_addr() as u64);
        }
        pages.push(page);
    }
    dcbaa.set(0, scratch_array_dma.phys_addr() as u64);
    (Some(scratch_array_dma), pages)
}

fn configure_endpoint_input_ctx(
    input_ctx: &ContextBlob,
    parsed: &ParsedConfig,
    speed: u32,
    port: u8,
    ep0_dma_phys: u64,
    ep0_mps: u32,
) -> Vec<(u8, DmaRegion, IntMutex<ProducerRing>)> {
    input_ctx.zero();

    let mut new_rings: Vec<(u8, DmaRegion, IntMutex<ProducerRing>)> = vec![];
    let mut max_dci: u32 = 1;
    // Per xHCI  6.2.5.1: A1 (EP0 add) shall NOT be asserted for Configure
    // Endpoint — only for Address Device. A0 (slot) is set because we update
    // context_entries when adding new endpoints.
    let mut add_flags: u32 = 0x01;

    for (_iface, eps) in &parsed.interfaces {
        for ep in eps {
            let dci = ep.dci();
            if !(2..=31).contains(&dci) {
                continue;
            }
            max_dci = max_dci.max(dci as u32);
            add_flags |= 1u32 << dci;
            let dma = DmaRegion::new(1);
            let ring = ProducerRing::new(&dma);
            new_rings.push((dci, dma, IntMutex::new(ring)));
        }
    }

    input_ctx.write_dword(0, 0, 0);
    input_ctx.write_dword(0, 1, add_flags);
    input_ctx.write_dword(1, 0, slot_context_dw0(0, speed, max_dci));
    input_ctx.write_dword(1, 1, slot_context_dw1(port));
    for (i, &dw) in ep0_context_dwords(ep0_mps, ep0_dma_phys, true)
        .iter()
        .enumerate()
    {
        input_ctx.write_dword(2, i, dw);
    }

    for (dci, dma, _ring) in &new_rings {
        let ep_desc: Option<&EndpointDescriptor> = parsed
            .interfaces
            .iter()
            .flat_map(|(_, eps)| eps.iter())
            .find(|ep| ep.dci() == *dci);
        if let Some(ep) = ep_desc {
            let dws = endpoint_context_dwords(ep, speed, dma.phys_addr() as u64, true);
            let ctx_idx = (*dci) as usize + 1;
            for (i, &dw) in dws.iter().enumerate() {
                input_ctx.write_dword(ctx_idx, i, dw);
            }
        }
    }

    new_rings
}
