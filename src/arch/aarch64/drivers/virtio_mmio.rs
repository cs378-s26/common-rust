use crate::print::kprintln;

use crate::arch::aarch64::device_tree::DeviceInfo;

// from the virtio spec: the magic value at offset 0x000 must always be this
const VIRTIO_MAGIC: u32 = 0x74726976; // "virt" in little-endian

// virtio-mmio register offsets (all 32-bit reads)
const OFFSET_MAGIC:     usize = 0x000;
const OFFSET_VERSION:   usize = 0x004;
const OFFSET_DEVICE_ID: usize = 0x008;

// reads a 32-bit virtio-mmio register at the given byte offset from the mapped base address
unsafe fn read_reg(base: usize, offset: usize) -> u32 {
    let ptr = (base + offset) as *const u32;
    unsafe { ptr.read_volatile() }
}

// maps a virtio DeviceID number to a human-readable name
fn device_id_name(id: u32) -> &'static str {
    match id {
        0  => "invalid",
        1  => "network",
        2  => "block",
        3  => "console",
        4  => "entropy (rng)",
        5  => "memory balloon",
        8  => "scsi host",
        9  => "9p transport",
        16 => "gpu",
        18 => "input",
        19 => "socket",
        34 => "sound",
        40 => "video",
        _  => "unknown",
    }
}

pub fn virtio_mmio_init(dev: &mut DeviceInfo) {
    let base = match dev.mmio_virt {
        Some(addr) => addr,
        None => {
            kprintln!("virtio_mmio: {} has no virtual mapping, skipping", dev.name);
            return;
        }
    };

    // safety: base came from our own MMIO mapping in init_device_discovery
    let magic = unsafe { read_reg(base, OFFSET_MAGIC) };
    if magic != VIRTIO_MAGIC {
        kprintln!(
            "virtio_mmio: {} bad magic {:#010x}, expected {:#010x}, skipping",
            dev.name, magic, VIRTIO_MAGIC
        );
        return;
    }

    let version = unsafe { read_reg(base, OFFSET_VERSION) };
    if version != 1 && version != 2 {
        kprintln!("virtio_mmio: {} unknown version {}, proceeding anyway", dev.name, version);
    }
    dev.virtio_version = Some(version);

    let device_id = unsafe { read_reg(base, OFFSET_DEVICE_ID) };
    dev.virtio_device_id = Some(device_id);

    kprintln!(
        "virtio_mmio: {} version={} device_id={} ({})",
        dev.name, version, device_id, device_id_name(device_id)
    );
}
