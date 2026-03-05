use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};



const QUEUE_CAP: usize = 64;

/// A decoded PS/2 mouse packet.
pub struct MousePacket {
    pub buttons: u8, // bit0=left, bit1=right, bit2=middle
    pub dx: i8,      // signed X delta
    pub dy: i8,      // signed Y delta (positive = up in PS/2 convention)
}

struct PacketQueue {
    buf: UnsafeCell<[MousePacket; QUEUE_CAP]>,
    head: AtomicUsize, // consumer index — only written by consumer
    tail: AtomicUsize, // producer index — only written by IRQ handler
}

unsafe impl Sync for PacketQueue {}

impl PacketQueue {
    const fn new() -> Self {
        Self {
            buf: UnsafeCell::new(unsafe { core::mem::zeroed() }),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Push a packet. Called only from IRQ context (single producer).
    /// Drops the packet silently if the queue is full.
    fn push(&self, pkt: MousePacket) {
        let tail = self.tail.load(Ordering::Relaxed);
        let next = (tail + 1) % QUEUE_CAP;
        if next == self.head.load(Ordering::Acquire) {
            return; // full — drop packet
        }
        unsafe { (*self.buf.get())[tail] = pkt };
        self.tail.store(next, Ordering::Release);
    }

    /// Pop a packet. Called from thread context (single consumer).
    fn pop(&self) -> Option<MousePacket> {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return None;
        }
        //head is only written here (single consumer).
        let pkt = unsafe { core::ptr::read(&(*self.buf.get())[head]) };
        self.head.store((head + 1) % QUEUE_CAP, Ordering::Release);
        Some(pkt)
    }
}

static PACKET_QUEUE: PacketQueue = PacketQueue::new();


// Index within the current 3-byte packet (0, 1, or 2).
static BYTE_IDX: AtomicUsize = AtomicUsize::new(0);

struct ScratchBuf(UnsafeCell<[u8; 3]>);
unsafe impl Sync for ScratchBuf {}
static SCRATCH: ScratchBuf = ScratchBuf(UnsafeCell::new([0u8; 3]));

pub fn enqueue_mouse_byte(byte: u8) {
    let idx = BYTE_IDX.load(Ordering::Relaxed);

    // Byte 0 must have bit 3 set per the PS/2 protocol; re-sync if not.
    if idx == 0 && (byte & 0x08) == 0 {
        return;
    }

    //single producer; idx is always 0, 1, or 2.
    unsafe { (*SCRATCH.0.get())[idx] = byte };

    if idx == 2 {
        // Complete packet — assemble and enqueue.
        let raw = unsafe { *SCRATCH.0.get() };
        PACKET_QUEUE.push(MousePacket {
            buttons: raw[0] & 0x07,
            dx: raw[1] as i8,
            dy: raw[2] as i8,
        });
        BYTE_IDX.store(0, Ordering::Relaxed);
    } else {
        BYTE_IDX.store(idx + 1, Ordering::Relaxed);
    }
}

/// Dequeue the next assembled mouse packet. Called from thread context.
pub fn dequeue_mouse_packet() -> Option<MousePacket> {
    PACKET_QUEUE.pop()
}

