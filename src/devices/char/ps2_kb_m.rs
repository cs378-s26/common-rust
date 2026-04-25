use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use ps2::{Controller, error::ControllerError, flags::ControllerConfigFlags};
use spin::Mutex;
use crate::arch::apic;
use crate::thread::{ThreadQueue, new_thread_queue};
use alloc::boxed::Box;

use crate::print::kprintln;

const QUEUE_CAP: usize = 256;

// Lock-free SPSC ring buffer. IRQ handler is the sole producer.
struct ScancodeQueue {
    buf: UnsafeCell<[u8; QUEUE_CAP]>,
    head: AtomicUsize, // consumer index — only written by consumer
    tail: AtomicUsize, // producer index — only written by IRQ handler
}

unsafe impl Sync for ScancodeQueue {}

impl ScancodeQueue {
    const fn new() -> Self {
        Self {
            buf: UnsafeCell::new([0u8; QUEUE_CAP]),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Push a byte. Called only from IRQ context (single producer).
    /// Drops the incoming byte silently if the queue is full.
    fn push(&self, val: u8) {
        let tail = self.tail.load(Ordering::Relaxed);
        let next = (tail + 1) % QUEUE_CAP;

        if next == self.head.load(Ordering::Acquire) {
            return; // full — drop byte; never write head from producer side
        }
        unsafe { (*self.buf.get())[tail] = val };
        self.tail.store(next, Ordering::Release);
    }

    /// Pop a byte. Called from thread context (single consumer).
    fn pop(&self) -> Option<u8> {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: head is only written here (single consumer).
        let val = unsafe { (*self.buf.get())[head] };
        self.head.store((head + 1) % QUEUE_CAP, Ordering::Release);
        Some(val)
    }
}

/* 
use alloc::vec::Vec;
//wait this doesn't work, it isn't O(1)
struct KbReq<T : const u32> {
    buf : Vec<u8>,
    remaining : usize
}

impl KbReq {
    fn new(len: usize) -> Self {
        Self {
            buf: Vec::new(),
            remaining: len,
        }
    }
    fn add_scancode(&mut self, scancode: u8) {
        if self.remaining > 0 {
            self.buf.push(scancode);
            self.remaining -= 1;
        }
    }
}

static KEYBOARD_REQ_QUEUE: Mutex<Vec<KbReq>> = Mutex::new(Vec::new());
static BLOCKED_READERS : Mutex<ThreadQueue> = Mutex::new(new_thread_queue());

// Reads new keystrokes from keyboard. 
fn add_kb_req(len : usize) -> Vec<u8> {

}

pub fn read(len: usize) -> Vec<u8> {

}

struct PS2Kb {

}
static SCANCODE_QUEUE: ScancodeQueue = ScancodeQueue::new();

pub fn enqueue_scancode(scancode: u8) {
    SCANCODE_QUEUE.push(scancode);
}

pub fn dequeue_scancode() -> Option<u8> {
    SCANCODE_QUEUE.pop()
}
*/
/// Lookup table: Set 2 make code → (unshifted char, shifted char).
/// 0 means no printable character for that code.
#[rustfmt::skip]
const SET2_MAP: [(u8, u8); 0x80] = [
    // 0x00
    (0,0),(0,0),(0,0),(0,0),(0,0),(0,0),(0,0),(0,0),
    // 0x08
    (0,0),(0,0),(0,0),(0,0),(0,0),(b'\t',b'\t'),(b'`',b'~'),(0,0),
    // 0x10 (0x11=LAlt, 0x12=LShift, 0x13=|, 0x14=LCtrl, 0x15=Q)
    (0,0),(0,0),(0,0),(b'\\',b'|'),(0,0),(b'q',b'Q'),(b'1',b'!'),(0,0),
    // 0x18
    (0,0),(0,0),(b'z',b'Z'),(b's',b'S'),(b'a',b'A'),(b'w',b'W'),(b'2',b'@'),(0,0),
    // 0x20
    (0,0),(b'c',b'C'),(b'x',b'X'),(b'd',b'D'),(b'e',b'E'),(b'4',b'$'),(b'3',b'#'),(0,0),
    // 0x28
    (0,0),(b' ',b' '),(b'v',b'V'),(b'f',b'F'),(b't',b'T'),(b'r',b'R'),(b'5',b'%'),(0,0),
    // 0x30
    (0,0),(b'n',b'N'),(b'b',b'B'),(b'h',b'H'),(b'g',b'G'),(b'y',b'Y'),(b'6',b'^'),(0,0),
    // 0x38
    (0,0),(0,0),(b'm',b'M'),(b'j',b'J'),(b'u',b'U'),(b'7',b'&'),(b'8',b'*'),(0,0),
    // 0x40
    (0,0),(b',',b'<'),(b'k',b'K'),(b'i',b'I'),(b'o',b'O'),(b'0',b')'),(b'9',b'('),(0,0),
    // 0x48
    (0,0),(b'.',b'>'),(b'/',b'?'),(b'l',b'L'),(b';',b':'),(b'p',b'P'),(b'-',b'_'),(0,0),
    // 0x50
    (0,0),(0,0),(b'\'',b'"'),(0,0),(b'[',b'{'),(b'=',b'+'),(0,0),(0,0),
    // 0x58 (0x58=CapsLock, 0x59=RShift, 0x5A=Enter, 0x5B=], 0x5D=\)
    (0,0),(0,0),(b'\n',b'\n'),(b']',b'}'),(0,0),(b'\\',b'|'),(0,0),(0,0),
    // 0x60
    (0,0),(0,0),(0,0),(0,0),(0,0),(0,0),(b'\x08',b'\x08'),(0,0), // 0x66=Backspace
    // 0x68 (numpad)
    (0,0),(b'1',b'1'),(0,0),(b'4',b'4'),(b'7',b'7'),(0,0),(0,0),(0,0),
    // 0x70 (numpad)
    (b'0',b'0'),(b'.',b'.'),(b'2',b'2'),(b'5',b'5'),(b'6',b'6'),(b'8',b'8'),(0,0),(0,0),
    // 0x78 (numpad)
    (0,0),(b'+',b'+'),(b'3',b'3'),(b'-',b'-'),(b'*',b'*'),(b'9',b'9'),(0,0),(0,0),
];

const SC_LSHIFT: u8 = 0x12;
const SC_RSHIFT: u8 = 0x59;
const SC_RELEASE: u8 = 0xF0;
const SC_EXTENDED: u8 = 0xE0;

struct DecoderState {
    release: bool,  // next byte is a key-release code
    extended: bool, // next byte is an extended (0xE0) key code
    shift: bool,    // a shift key is currently held
}

impl DecoderState {
    const fn new() -> Self {
        Self {
            release: false,
            extended: false,
            shift: false,
        }
    }
}

static DECODER: Mutex<DecoderState> = Mutex::new(DecoderState::new());

/// Feed one raw byte from the PS/2 data port into the decoder.
pub fn decode_scancode(byte: u8) -> Option<char> {
    let mut st = DECODER.lock();

    match byte {
        SC_RELEASE => {
            st.release = true;
            return None;
        }
        SC_EXTENDED => {
            st.extended = true;
            return None;
        }
        _ => {}
    }

    let is_release = core::mem::replace(&mut st.release, false);
    let is_extended = core::mem::replace(&mut st.extended, false);

    // Extended keys (arrows, ins/del, etc.) — nothing printable for now.
    if is_extended {
        return None;
    }

    // Track shift state.
    if byte == SC_LSHIFT || byte == SC_RSHIFT {
        st.shift = !is_release;
        return None;
    }

    if is_release {
        return None;
    }

    if byte < 0x80 {
        let (lo, hi) = SET2_MAP[byte as usize];
        let ch = if st.shift { hi } else { lo };
        if ch != 0 {
            return Some(ch as char);
        }
    }
    None
}

//  PS/2 controller initialization

fn map_err(e: ControllerError) -> &'static str {
    match e {
        ControllerError::Timeout => "timeout",
        ControllerError::TestFailed { .. } => "self-test failed",
    }
}

pub fn try_get_data() -> Option<u8> {
    let status = unsafe { x86::io::inb(0x64) };
    if status & 0x1 == 0 {
        None
    } else {
        Some(unsafe { x86::io::inb(0x60) })
    }
}

pub fn reset() {
    wait_until_ready();
    unsafe { x86::io::outb(0x64, 0xFE) };
}

fn keyboard_irq_handler() -> Option<()> {
    apic::eoi();
    let scancode: u8 = unsafe { x86::io::inb(0x60) };
    //enqueue_scancode(scancode);
    if let Some(ch) = decode_scancode(scancode) {
        kprintln!("Got char: {}", ch);
    }
    Some(())
}

fn wait_until_response() {
    loop {
        let status = unsafe { x86::io::inb(0x64) };
        if status & 0x1 == 1 {
            break;
        }
    }
}

fn wait_until_ready() {
    loop {
        let status = unsafe { x86::io::inb(0x64) };
        if status & 0x2 == 0 {
            break;
        }
    }
}


/// Initialize both PS/2 devices (keyboard + mouse) following the OSDev wiki sequence.
/// Must be called once on the BSP before enabling IRQs.
/// https://docs.rs/ps2/latest/ps2/
/// https://wiki.osdev.org/%228042%22_PS/2_Controller#Initialising_the_PS/2_Controller
pub fn init_ps2() -> Result<(), &'static str> {
    let mut ctrl = unsafe { Controller::new() };

    ctrl.disable_keyboard()
        .map_err(|_| "disable_keyboard failed")?;
    ctrl.disable_mouse().ok();
    kprintln!("[PS2] ports disabled");

    while ctrl.read_data().is_ok() {}
    kprintln!("[PS2] output buffer flushed");

    let mut config = ctrl.read_config().map_err(|_| "read_config failed")?;
    config.set(
        ControllerConfigFlags::ENABLE_KEYBOARD_INTERRUPT
            | ControllerConfigFlags::ENABLE_MOUSE_INTERRUPT
            | ControllerConfigFlags::ENABLE_TRANSLATE,
        false,
    );
    ctrl.write_config(config)
        .map_err(|_| "write_config failed")?;

    ctrl.test_controller().map_err(map_err)?;
    ctrl.write_config(config).ok();
    kprintln!("[PS2] controller self-test: PASS");

    let has_mouse = {
        ctrl.enable_mouse().is_ok() && {
            let c = ctrl.read_config().unwrap_or(config);
            let present = !c.contains(ControllerConfigFlags::DISABLE_MOUSE);
            ctrl.disable_mouse().ok();
            present
        }
    };
    kprintln!("[PS2] mouse present: {}", has_mouse);

    let keyboard_works = ctrl.test_keyboard().is_ok();
    let mouse_works = has_mouse && ctrl.test_mouse().is_ok();
    kprintln!(
        "[PS2] keyboard port test: {}",
        if keyboard_works { "PASS" } else { "FAIL" }
    );
    kprintln!(
        "[PS2] mouse port test: {}",
        if mouse_works { "PASS" } else { "FAIL" }
    );

    config = ctrl.read_config().map_err(|_| "read_config (2) failed")?;

    if keyboard_works {
        ctrl.enable_keyboard()
            .map_err(|_| "enable_keyboard failed")?;
        config.set(ControllerConfigFlags::DISABLE_KEYBOARD, false);
        config.set(ControllerConfigFlags::ENABLE_KEYBOARD_INTERRUPT, true);
        ctrl.write_config(config).map_err(|_| "write_config (keyboard) failed")?;
        ctrl.keyboard()
            .reset_and_self_test()
            .map_err(|_| "keyboard reset/self-test failed")?;
        kprintln!("[PS2] keyboard reset+self-test: PASS");
        // QEMU auto-enables scanning after reset; treat failure as non-fatal.
        
        match ctrl.keyboard().enable_scanning() {
            Ok(()) => kprintln!("[PS2] keyboard scanning enabled"),
            Err(_) => kprintln!("[PS2] enable_scanning: failed (non-fatal)"),
        }
        ctrl.keyboard()
            .reset_and_self_test()
            .map_err(|_| "keyboard reset/self-test failed")?;
        crate::arch::register_irq_handler(1, Box::new(keyboard_irq_handler), None);
        kprintln!("PS2 keyboard IRQ handler registered");
    }
    
    //for some reason, enabling mouse makes keyboard stop working
    /* 
    if false {
        ctrl.enable_mouse().map_err(|_| "enable_mouse failed")?;
        config.set(ControllerConfigFlags::DISABLE_MOUSE, false);
        config.set(ControllerConfigFlags::ENABLE_MOUSE_INTERRUPT, true);
        ctrl.write_config(config).map_err(|_| "write_config (mouse) failed")?;
        ctrl.mouse()
            .reset_and_self_test()
            .map_err(|_| "mouse reset/self-test failed")?;
        kprintln!("[PS2] mouse reset+self-test: PASS");
        match ctrl.mouse().enable_data_reporting() {
            Ok(()) => kprintln!("[PS2] mouse data reporting enabled"),
            Err(_) => kprintln!("[PS2] enable_data_reporting: failed (non-fatal)"),
        }
        /* 
        crate::arch::register_irq_handler(12, Box::new(|| {
            apic::eoi();
            kprintln!("Mouse IRQ handler called!");
            Some(())    }), Some(0x68));
        */
    }
    */

    // Write final config  this enables IRQs for all working devices.
    kprintln!(
        "[PS2] init complete (kbd={} mouse={})",
        keyboard_works,
        mouse_works
    );
    //reset again?
    Ok(())
}
