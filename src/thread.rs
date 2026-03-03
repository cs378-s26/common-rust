extern crate alloc;

use core::{
    cell::{Cell, OnceCell, RefCell, RefMut},
    ffi::c_void,
    pin::Pin,
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(target_arch = "x86_64")]
use core::arch::naked_asm;

use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use spin::{Mutex, MutexGuard, Once};

use crate::{
    arch::{
        Arch, ArchTrait, Context, ContextTrait, IPI_WAKE_VECTOR, InterruptContext, IrqState, apic,
        sleep_core,
    },
    local_storage::{LocalStorage, LocalStorageHandler, impl_local_storage},
    mp::{MP_STAGE, MPStage, core_local},
    sync::MutexLike,
};

pub struct Thread {
    pub link: LinkedListAtomicLink,
    #[allow(unused)]
    pub tls: Pin<Box<[u8]>>,
    pub tls_addr: u64, // aliased to tls
}

impl Thread {
    pub fn new() -> Arc<Thread> {
        let tls = ThreadLocalStorageHandler::create();
        let tls_addr = Box::as_ptr(&tls).as_ptr() as u64;

        let handle = Arc::new(Thread {
            link: LinkedListAtomicLink::new(),
            tls: Pin::new(tls),
            tls_addr,
        });

        THIS_THREAD
            .read_for(&handle)
            .call_once(|| Arc::downgrade(&handle));

        // TODO: relax memory ordering here
        TID.read_for(&handle)
            .store(CURR_TID.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);

        handle
    }

    pub fn is_same_thread(lhs: &Arc<Thread>, rhs: &Arc<Thread>) -> bool {
        Arc::as_ptr(lhs) == Arc::as_ptr(rhs)
    }

    pub fn tid(&self) -> u64 {
        TID.read_for(self).load(Ordering::Relaxed)
    }

    pub fn this_tid() -> u64 {
        TID.load(Ordering::Relaxed)
    }
}

// TLS

unsafe extern "C" {
    static _marker_thread_local_template_start: c_void;
    static _marker_thread_local_template_end: c_void;
}

pub struct ThreadLocalStorageHandler;

impl LocalStorageHandler for ThreadLocalStorageHandler {
    fn get_range() -> (*const c_void, *const c_void) {
        unsafe {
            (
                &_marker_thread_local_template_start,
                &_marker_thread_local_template_end,
            )
        }
    }

    fn get_base() -> u64 {
        assert!(is_on_thread());
        unsafe { Arch::get_thread_local_pointer() }
    }
}

impl<T> ThreadLocal<T> {
    pub const fn new(val: T) -> Self {
        Self(val)
    }
}

impl<T: Send + Sync> ThreadLocal<T> {
    pub fn read_for(&self, thread: &Thread) -> &T {
        unsafe { &*((thread.tls_addr + self.offset()) as *const T) }
    }
}

impl_local_storage!(ThreadLocal, ThreadLocalStorageHandler);

#[repr(C)]
pub struct ThreadLocal<T>(T);

pub macro thread_local {
    {
        $(
            $(#[$meta:meta])*
            $vis:vis $name:ident : $ty:ty = $init:expr;
        )*
    } => {
        $(
            $(#[$meta])*
            #[unsafe(link_section = ".thread_local")]
            $vis static $name: crate::thread::ThreadLocal<$ty> = crate::thread::ThreadLocal::new($init);
        )*
    }
}

// queue types

intrusive_adapter!(pub ThreadQueueAdapter = Arc<Thread>: Thread { link => LinkedListAtomicLink });

pub type ThreadQueue = LinkedList<ThreadQueueAdapter>;

pub const fn new_thread_queue() -> ThreadQueue {
    ThreadQueue::new(ThreadQueueAdapter::NEW)
}

// thread scheduling and management

// TODO: alignment here is ABI specific, this needs to be moved into src/arch
#[repr(align(16))]
struct Stack([u8; 4 * 4096]);

core_local! {
    pub IDLE: OnceCell<Arc<Thread>> = OnceCell::new();
    CURRENT_THREAD: Cell<Option<Arc<Thread>>> = Cell::new(None);
    CTX_SWITCH_STACK: Stack = Stack([0; _]);
    LOCAL_WORK_QUEUE: RefCell<ThreadQueue> = RefCell::new(new_thread_queue());
}

thread_local! {
    pub THIS_THREAD: Once<Weak<Thread>> = Once::new();
    pub CONTEXT: Mutex<Context> = Mutex::new(Default::default());
    pub CAN_YIELD: AtomicBool = AtomicBool::new(false);
    TID: AtomicU64 = AtomicU64::new(0);
    STACK: Stack = Stack([0; _]);
    CTX_GUARD: Cell<Option<MutexGuard<'static, Context>>> = Cell::new(None);
}

static CURR_TID: AtomicU64 = AtomicU64::new(1);

static GLOBAL_WORK_QUEUE: Mutex<ThreadQueue> = Mutex::new(new_thread_queue());

pub fn local_work_queue() -> RefMut<'static, ThreadQueue> {
    LOCAL_WORK_QUEUE.borrow_mut()
}

fn thread_enter(thread: Arc<Thread>) {
    unsafe { Arch::set_thread_local_pointer(&thread.tls_addr) };
    CURRENT_THREAD.set(Some(thread));
}

fn thread_exit() {
    CURRENT_THREAD.set(None);
    unsafe { Arch::set_thread_local_pointer(ptr::null()) };
}

pub fn is_on_thread() -> bool {
    let thread = CURRENT_THREAD.take();
    let res = thread.is_some();
    CURRENT_THREAD.set(thread);
    res
}

unsafe fn go_to_thread(thread: Arc<Thread>) -> ! {
    assert!(!is_on_thread());

    thread_enter(thread);

    let ctx = CONTEXT.lock();
    let state: Context = *ctx;
    CTX_GUARD.set(Some(ctx));

    state.jump_to();
}

pub fn this_thread() -> Arc<Thread> {
    THIS_THREAD.get()
        .unwrap()
        .upgrade().expect("this_thread, although weak, should never return None when upgrading and the current thread is running")
}

pub fn set_up_idle() -> Arc<Thread> {
    let Ok(_) = IDLE.set(Thread::new()) else {
        panic!("expected core-local idle to be not init");
    };

    thread_enter(IDLE.get().unwrap().clone());
    let mut ctx = CONTEXT.lock();
    ctx.setup_kthread_context();
    CTX_GUARD.set(Some(ctx));

    IDLE.get().unwrap().clone()
}

pub fn poll_tasks() -> ! {
    assert!(
        Thread::is_same_thread(&this_thread(), IDLE.get().unwrap()),
        "poll_tasks may only be called from idle"
    );

    // Ensure IRQs are enabled so sleep_core (hlt) can be woken by IPIs/timer.
    Arch::set_irq_enabled(true);

    loop {
        loop {
            let Some(thread) = local_work_queue().pop_front() else {
                break;
            };

            GLOBAL_WORK_QUEUE.lock().push_back(thread);
        }

        let thread = {
            let mut lock = GLOBAL_WORK_QUEUE.lock();
            let task = lock.pop_front();
            drop(lock);
            task
        };

        let Some(thread) = thread else {
            sleep_core();
            continue;
        };

        suspend_to_thread(thread);
    }
}

pub fn can_yield() -> bool {
    // we can only yield if we are in a thread context,
    // the current thread can yield (for instance, idle cannot yield),
    // and interrupts are enabled (if IRQs are disabled we're likely in an
    // interrupt handler or critical section).
    Arch::irq_is_enabled() && can_yield_for_preempt()
}

// Like `can_yield` but skips the IRQ-enabled check.
// we call this from the timer ISR where IRQs are already disabled by the CPU,
// but we still want to preempt.
pub fn can_yield_for_preempt() -> bool {
    MP_STAGE.load(Ordering::Relaxed) == MPStage::MPPreempt
        && is_on_thread()
        && CAN_YIELD.load(Ordering::Relaxed)
}

/// Handles preemption. Resumes execution on the target thread.
/// # Safety
/// Can only be called from IRQ
pub unsafe fn preempt_to(ctx: &InterruptContext, target: Arc<Thread>) -> ! {
    assert!(can_yield_for_preempt());

    // Save the interrupted context and release the CONTEXT lock.
    let mut guard = CTX_GUARD
        .take()
        .expect("CTX_GUARD not set during preemption");
    guard.save_from_interrupt(ctx);
    drop(guard);

    let thread = THIS_THREAD.get().expect("THIS_THREAD not set").clone();

    thread_exit();

    GLOBAL_WORK_QUEUE
        .lock()
        .push_back(thread.upgrade().unwrap());
    apic::send_ipi_all_except_self(IPI_WAKE_VECTOR);

    unsafe { go_to_thread(target) }
}

/// Preempt to the idle thread, for general purpose rescheduling.
/// # Safety
/// Can only be called from IRQ
pub unsafe fn preempt_to_idle(ctx: &InterruptContext) -> ! {
    unsafe { preempt_to(ctx, IDLE.get().unwrap().clone()) }
}

// flowey writes "worst function in mos history" asked to drop the class
fn suspend_impl<T: FnOnce(Arc<Thread>)>(action: T, target: Arc<Thread>) {
    let irq_state = IrqState::save();
    Arch::set_irq_enabled(false);

    assert!(
        is_on_thread(),
        "yield_thread_with_action_to() called when the current core is not in a thread context"
    );

    let thread = this_thread();
    let context = CTX_GUARD.take().expect("CTX_GUARD not set");

    unsafe {
        Arch::save_context(&(*CTX_SWITCH_STACK).0, context, move || {
            thread_exit();
            action(thread);
            go_to_thread(target);
        })
    };

    irq_state.restore();
}

#[inline(always)]
pub fn suspend_to_queue<T: MutexLike<ThreadQueue>>(queue: &T) {
    // We need to lock *before* suspend_impl, because interrupts are blocked there, and we shouldn't deal with
    // that when in the limbo state.
    let mut queue = queue.lock();

    suspend_impl(
        move |t| {
            queue.push_back(t);
            drop(queue);
        },
        IDLE.get().unwrap().clone(),
    );
}

#[inline(always)]
pub fn suspend_to_thread(thread: Arc<Thread>) {
    suspend_impl(drop, thread);
}

#[inline(always)]
pub fn yield_thread() {
    suspend_to_queue(&GLOBAL_WORK_QUEUE);
}

pub fn make_thread<T: FnOnce() + Send + 'static>(task: T) -> Arc<Thread> {
    unsafe extern "C" fn thread_entry<T: FnOnce()>(task: *mut T) -> ! {
        {
            let task = unsafe { Box::from_raw(task) };
            task();
        }

        suspend_to_thread(IDLE.get().unwrap().clone());
        unreachable!()
    }

    #[cfg(target_arch = "x86_64")]
    #[unsafe(naked)]
    unsafe extern "C" fn thread_entry0<T: FnOnce()>(task: *mut T) -> ! {
        // SysV ABI: emulate a call frame so the callee entry stack layout is what Rust expects.
        naked_asm!(
            "pushq $0",
            "jmp {0}",
            sym thread_entry::<T>,
            options(att_syntax)
        )
    }

    #[cfg(target_arch = "aarch64")]
    unsafe extern "C" fn thread_entry0<T: FnOnce()>(task: *mut T) -> ! {
        unsafe { thread_entry(task) }
    }

    let thread = Thread::new();

    {
        let task = Box::into_raw(Box::new(task));
        let mut ctx = CONTEXT.read_for(&thread).lock();
        *ctx = Context::new_kthread(&STACK.read_for(&thread).0, thread_entry0, task);
    }

    CAN_YIELD.read_for(&thread).store(true, Ordering::Relaxed);
    thread.clone()
}

pub fn spawn_thread<T: FnOnce() + Send + 'static>(task: T) {
    let thread = make_thread(task);
    GLOBAL_WORK_QUEUE.lock().push_back(thread);
    apic::send_ipi_all_except_self(IPI_WAKE_VECTOR);
}
