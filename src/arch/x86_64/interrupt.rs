use core::{
    arch::naked_asm,
    sync::atomic::{AtomicU64, Ordering},
};

use x86::controlregs::cr2;
use x86_64::structures::idt::PageFaultErrorCode;
use intrusive_collections::{KeyAdapter, LinkedList, LinkedListAtomicLink, intrusive_adapter};
use alloc::{vec, vec::Vec, boxed::Box, rc::Rc};
use intrusive_collections::{RBTree, RBTreeLink};
use crate::sync::{IntMutex, MutexLike, IntSpinLock};
use crate::Once;

use super::apic;
use crate::{
    event::{Event::PageFault, push_event},
    memory::virtual_memory::PageFaultConditions,
    mp::CORE_ID,
    thread::this_thread,
};

static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

#[repr(C)]
pub struct InterruptContext {
    pub regs: [u64; 14],
    pub rbp: u64, // For preemptive context restore.
    pub id: u64,
    pub err: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

// IDT magic

const fn error_code_offset(int_no: u8) -> u64 {
    if int_no == 8 || (10..=14).contains(&int_no) || int_no == 17 || int_no == 21 {
        0
    } else {
        8
    }
}

#[unsafe(naked)]
pub(super) unsafe extern "C" fn irq_handler_entry<const I: u8>() -> ! {
    naked_asm!(
        // required for ABI reasons
        "cld",

        // normalize the stack frame: [int#, ec]
        "subq ${}, %rsp",
        "pushq ${}",
        "jmp {}",

        options(att_syntax),
        const error_code_offset(I),
        const I,
        sym irq_handler_t0
    )
}

#[unsafe(naked)]
unsafe extern "C" fn irq_handler_t0() -> ! {
    naked_asm!(
        "pushq %rbp",
        "pushq %rax",
        "pushq %rcx",
        "pushq %rdx",
        "pushq %rbx",
        "pushq %rsi",
        "pushq %rdi",
        "pushq %r8",
        "pushq %r9",
        "pushq %r10",
        "pushq %r11",
        "pushq %r12",
        "pushq %r13",
        "pushq %r14",
        "pushq %r15",

        // point to top of stack (1st arg: InterruptContext*)
        "movq %rsp, %rdi",

        // simulate the call frame
        "pushq $0",
        "pushq %rbp",
        "movq %rsp, %rbp",

        // align stack
        "andq $~15, %rsp",

        // invoke
        "call {}",

        "movq %rbp, %rsp",
        "popq %rbp",
        "addq $8, %rsp",

        "popq %r15",
        "popq %r14",
        "popq %r13",
        "popq %r12",
        "popq %r11",
        "popq %r10",
        "popq %r9",
        "popq %r8",
        "popq %rdi",
        "popq %rsi",
        "popq %rbx",
        "popq %rdx",
        "popq %rcx",
        "popq %rax",
        "popq %rbp",

        "addq $16, %rsp",
        "iretq",
        options(att_syntax),
        sym irq_handler_t1
    );
}

pub mod irq_vector {
    pub const PAGE_FAULT: u8 = 0x0e;
    pub const TIMER_INTERRUPT: u8 = 0x20;
    pub const IPI_WAKE: u8 = 0x21;
    pub const TLB_SHOOTDOWN: u8 = 0x22;
}

pub extern "C" fn timer_interrupt_handler(ctx: &InterruptContext) {
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
    apic::eoi();

    unsafe { crate::thread::preempt_to_idle(ctx) };
}

pub extern "C" fn ipi_wake_handler(_ctx: &InterruptContext) {
    apic::eoi();
}

unsafe extern "C" fn irq_handler_t1(addr: *mut InterruptContext) {
    let context = unsafe { &*addr };
    use irq_vector::*;
    match context.id as u8 {
        PAGE_FAULT => {
            if let Some(code) = PageFaultErrorCode::from_bits(context.err) {
                // seems like kind of a lot of overhead for interface translation...
                let mut cause = PageFaultConditions::empty();
                if code.contains(PageFaultErrorCode::PROTECTION_VIOLATION) {
                    cause.insert(PageFaultConditions::PRESENT);
                }
                if code.contains(PageFaultErrorCode::CAUSED_BY_WRITE) {
                    cause.insert(PageFaultConditions::WRITE);
                }
                if code.contains(PageFaultErrorCode::USER_MODE) {
                    cause.insert(PageFaultConditions::USER);
                }
                if code.contains(PageFaultErrorCode::MALFORMED_TABLE) {
                    cause.insert(PageFaultConditions::CORRUPT);
                }
                if code.contains(PageFaultErrorCode::INSTRUCTION_FETCH) {
                    cause.insert(PageFaultConditions::FETCH);
                }
                push_event(
                    PageFault {
                        cause,
                        address: unsafe { cr2() },
                        thread: this_thread(),
                    },
                    CORE_ID.get(),
                );
                unsafe { crate::thread::block_to_idle(context) };
            } else {
                panic!("hi: {} #{}, cr2={}", context.err, context.id, unsafe {
                    cr2()
                });
            }
        }
        TIMER_INTERRUPT => timer_interrupt_handler(context),
        IPI_WAKE => ipi_wake_handler(context),
        TLB_SHOOTDOWN => {
            apic::eoi();
            unsafe { crate::thread::preempt_to_idle(context) };
        }
        _ => panic!(
            "Unhandled interrupt #{}: err={}, cr2={:x}",
            context.id,
            context.err,
            unsafe { cr2() }
        ),
    }
}


//this is hardcoded since apparently x86-64 has only 256 interrupt vectors
static mut occupied_vectors: IntMutex<[bool; 256]> = 
    IntMutex::new([false; 256]);
static mut next_vector : IntMutex<u8> = IntMutex::new(0x30); // start at 0x30 to avoid conflicts with exceptions

/*
* Design: 
*/

intrusive_adapter!(InterruptHandlerAdapter = Arc<InterruptHandler>: InterruptHandler { link => LinkedListAtomicLink });
struct InterruptHandler {
    handler : Box<dyn (Fn() -> Option<()>) + Send + Sync>,
    link : LinkedListAtomicLink,
}

use core::cell::RefCell;

struct InterruptHandlersLine {
    irq : u8,
    handlers : RefCell<LinkedList<InterruptHandlerAdapter>>,
    link : RBTreeLink,
}

impl<'a> KeyAdapter<'a> for InterruptHandlersLineAdapter {
    type Key = u8;
    fn get_key(&self, value: &'a InterruptHandlersLine) -> u8 {
        value.irq
    }
}

intrusive_adapter!(InterruptHandlersLineAdapter = Rc<InterruptHandlersLine>: InterruptHandlersLine { link => RBTreeLink });

use alloc::sync::Arc;

static mut handlers : 
IntMutex<RBTree<InterruptHandlersLineAdapter>> = IntMutex::new(RBTree::new(InterruptHandlersLineAdapter::new()));

pub fn register_irq_handler(irq_num : u8, handler : Box<dyn (Fn() -> Option<()>) + Send + Sync>) {
    let handler = Arc::new(InterruptHandler { handler, link: LinkedListAtomicLink::new() });
    let mut handlers_list = unsafe {handlers.lock() };
    let handlers_for_line = handlers_list.find_mut(&irq_num);
    if let Some(true_list) = handlers_for_line.get() {
        true_list.handlers.borrow_mut().push_back(handler);
    } else {
        let mut new_list = LinkedList::new(InterruptHandlerAdapter::new());
        new_list.push_back(handler);
        let new_line = InterruptHandlersLine { irq: irq_num, handlers: new_list, link: RBTreeLink::new() };
        handlers_list.insert(Rc::new(new_line));
    }
}

//TODO: store a mapping of irq vector --> irq number
fn handle_device_interrupt(irq_vec : u8) {
    let handlers_list = unsafe {handlers.lock() };
    let mut has_handled = false;
    if let Some(true_list) = handlers_list.find(&irq_num).get() {
        for handler in true_list.handlers.borrow().iter() {
            let irq_handler = &handler.handler;
            if let Some(_) = irq_handler() {
                has_handled = true;
                break;
            }
        }
    }
    if !has_handled {
        kprintln!("Spurious/unhandled interrupt on IRQ line {}", irq_num);
    }
}