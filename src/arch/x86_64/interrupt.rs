use core::{
    arch::naked_asm,
    sync::atomic::{AtomicU64, Ordering},
};

use x86::controlregs::cr2;
use intrusive_collections::{KeyAdapter, LinkedList, LinkedListAtomicLink, intrusive_adapter};
use alloc::{vec, vec::Vec, boxed::Box, rc::Rc};
use intrusive_collections::{RBTree, RBTreeLink};
use crate::sync::{IntMutex, MutexLike, IntSpinLock};
use crate::Once;
use x86_64::{registers::segmentation::GS, structures::idt::PageFaultErrorCode};

use super::apic;
use crate::{
    event::{Event::PageFault, push_event},
    memory::virtual_memory::PageFaultConditions,
    mp::CORE_ID,
    thread::{IDLE, suspend_to_thread, this_thread},
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
        sym irq_handler_t1,
    );
}

pub mod irq_vector {
    pub const PAGE_FAULT: u8 = 0x0e;
    pub const TIMER_INTERRUPT: u8 = 0x20;
    pub const IPI_WAKE: u8 = 0x21;
    pub const TLB_SHOOTDOWN: u8 = 0x22;
    pub const SYSCALL: u8 = 0x80;
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
    let from_user = context.cs & 0b11 == 3;
    if from_user {
        unsafe { GS::swap() };
        //reset FS
        let cur_thread = crate::thread::CURRENT_THREAD.take();
        if let Some(thread) = &cur_thread {
            unsafe { super::set_thread_local_pointer(&thread.tls_addr) };
        }
        crate::thread::CURRENT_THREAD.set(cur_thread);
    }
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
        SYSCALL => {
            // TODO: general purpose syscall handler
            // get rax
            let syscall_id = context.regs[13];

            this_thread()
                .process
                .get()
                .unwrap()
                .exit_code
                .set(syscall_id);
            suspend_to_thread(IDLE.get().unwrap().clone());
        }
        _ => {
            handle_device_interrupt(context.id as u8);
        },
    }
    if from_user {
        unsafe { GS::swap() };
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

use intrusive_collections::RBTreeAtomicLink;

struct InterruptHandlersLine {
    irq : u8,
    handlers : IntMutex<LinkedList<InterruptHandlerAdapter>>,
    link : RBTreeAtomicLink,
}

impl<'a> KeyAdapter<'a> for InterruptHandlersLineAdapter {
    type Key = u8;
    fn get_key(&self, value: &'a InterruptHandlersLine) -> u8 {
        value.irq
    }
}

intrusive_adapter!(InterruptHandlersLineAdapter = Arc<InterruptHandlersLine>: InterruptHandlersLine { link => RBTreeLink });

use alloc::sync::Arc;

static HANDLERS : 
IntMutex<RBTree<InterruptHandlersLineAdapter>> = IntMutex::new(RBTree::new(InterruptHandlersLineAdapter::NEW));

pub fn register_irq_handler(irq_num : u8, handler : Box<dyn (Fn() -> Option<()>) + Send + Sync>) {
    let gsi_num = crate::devices::discovery::acpi::get_gsi_for_irq(irq_num);
    //TODO: use MADT flags to determine trigger mode and polarity
    //for now, we'll just route IRQ to 0x67
    super::ioapic::route_irq(gsi_num, 0x67, apic::get_lapic_id() as u32);
    let handler = Arc::new(InterruptHandler { handler, link: LinkedListAtomicLink::new() });
    let mut handlers_list = HANDLERS.lock();
    let handlers_for_line = handlers_list.find_mut(&0x67);
    if let Some(true_list) = handlers_for_line.get() {
        true_list.handlers.lock().push_back(handler);
    } else {
        let mut new_list = LinkedList::new(InterruptHandlerAdapter::new());
        new_list.push_back(handler);
        let new_line = InterruptHandlersLine { irq: 0x67, handlers: IntMutex::new(new_list), link: RBTreeAtomicLink::new() };
        handlers_list.insert(Arc::new(new_line));
    }
}

//TODO: store a mapping of irq vector --> irq number
fn handle_device_interrupt(irq_vec : u8) {
    let handlers_list = HANDLERS.lock();
    let mut has_handled = false;
    if let Some(true_list) = handlers_list.find(&irq_vec).get() {
        for handler in true_list.handlers.lock().iter() {
            let irq_handler = &handler.handler;
            if let Some(_) = irq_handler() {
                has_handled = true;
                break;
            }
        }
    }
    if !has_handled {
        panic!("Spurious interrupt on IRQ {}", irq_vec);
    }
}