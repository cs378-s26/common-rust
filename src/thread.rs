extern crate alloc;

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
};
#[cfg(target_arch = "x86_64")]
use core::arch::naked_asm;
use core::{
    cell::{Cell, OnceCell},
    ffi::c_void,
    ops::DerefMut,
    pin::Pin,
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use intrusive_collections::{
    LinkedList, LinkedListAtomicLink, RBTreeAtomicLink, intrusive_adapter,
};
use spin::{Mutex, MutexGuard, Once};

use crate::{
    arch::{Arch, ArchTrait, Context, ContextTrait, InterruptContext},
    local_storage::{LocalStorage, LocalStorageHandler, impl_local_storage},
    memory::virtual_memory_2::VirtualMemory,
    mp::{CORE_ID, CoreId, MP_STAGE, MPStage, core_local},
    process::Process,
    state::{Irq, StateGuard},
    sync::{IntSpinLock, MutexLike},
};

pub struct Thread {
    // used for generally queuing threads somewhere
    pub link: LinkedListAtomicLink,

    // used for scheduling
    // some schedulers may opt to not use this (e.g. round robin)
    pub rb_link: RBTreeAtomicLink,

    #[allow(unused)]
    pub tls: Pin<Box<[u8]>>,
    pub tls_addr: u64, // aliased to tls
    pub process: Once<Arc<Process>>,
}

impl Thread {
    pub fn new() -> Arc<Thread> {
        let tls = ThreadLocalStorageHandler::create();
        let tls_addr = Box::as_ptr(&tls).as_ptr() as u64;

        let handle = Arc::new(Thread {
            link: LinkedListAtomicLink::new(),
            rb_link: RBTreeAtomicLink::new(),
            tls: Pin::new(tls),
            tls_addr,
            process: Once::new(),
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
struct Stack([u8; 64 * 4096]);

core_local! {
    pub IDLE: OnceCell<Arc<Thread>> = OnceCell::new();
    pub CURRENT_THREAD: Cell<Option<Arc<Thread>>> = Cell::new(None);
    CTX_SWITCH_STACK: Stack = Stack([0; _]);
    pub LOCAL_WORK_QUEUE: IntSpinLock<ThreadQueue> = IntSpinLock::new(new_thread_queue());
}

thread_local! {
    pub THIS_THREAD: Once<Weak<Thread>> = Once::new();
    pub CONTEXT: Mutex<Context> = Mutex::new(Default::default());
    pub CAN_YIELD: AtomicBool = AtomicBool::new(false);
    pub IS_IDLE: AtomicBool = AtomicBool::new(false);
    pub PINNED_TO_CORE: AtomicBool = AtomicBool::new(false);
    pub CORE_PINNED_TO: AtomicUsize = AtomicUsize::new(usize::MAX);
    TID: AtomicU64 = AtomicU64::new(0);
    STACK: Stack = Stack([0; _]);
    CTX_GUARD: Cell<Option<MutexGuard<'static, Context>>> = Cell::new(None);
}

static CURR_TID: AtomicU64 = AtomicU64::new(1);

static GLOBAL_WORK_QUEUE: IntSpinLock<ThreadQueue> = IntSpinLock::new(new_thread_queue());

fn thread_enter(thread: Arc<Thread>) {
    // assert!(!Arch::irq_is_enabled());

    unsafe { Arch::set_thread_local_pointer(&thread.tls_addr) };
    if let Some(process) = thread.process.get() {
        Arch::set_user_address_space(process.virtual_memory.get_page_table() as u64);
    } else {
        Arch::set_user_address_space(VirtualMemory::get_limine_page_table() as u64);
    }
    CURRENT_THREAD.set(Some(thread));
}

fn thread_exit() {
    // assert!(!Arch::irq_is_enabled());

    CURRENT_THREAD.set(None);
    unsafe { Arch::set_thread_local_pointer(ptr::null()) };
}

pub fn is_on_thread() -> bool {
    let _guard = StateGuard::<Irq>::guard();

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

    IS_IDLE.store(true, Ordering::Relaxed);

    IDLE.get().unwrap().clone()
}

pub fn poll_tasks() -> ! {
    assert!(
        Thread::is_same_thread(&this_thread(), IDLE.get().unwrap()),
        "poll_tasks may only be called from idle"
    );
    loop {
        if let Some(thread) = { LOCAL_WORK_QUEUE.lock().pop_front() } {
            if PINNED_TO_CORE.read_for(&thread).load(Ordering::Relaxed) {
                suspend_to_thread(thread); // TODO scheduled unfairly often
            } else {
                GLOBAL_WORK_QUEUE.lock().push_back(thread);
                Arch::wake_other_cores();
            }
        }
        if let Some(thread) = { GLOBAL_WORK_QUEUE.lock().pop_front() } {
            suspend_to_thread(thread);
        }
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
pub unsafe fn preempt_to(ctx: &InterruptContext, target: Arc<Thread>, requeue: bool) {
    // idle thread is allowed to call preempt_to
    if can_yield_for_preempt() || IS_IDLE.load(Ordering::Relaxed) {
        assert!(
            !Arch::irq_is_enabled(),
            "IRQ cannot be enabled on preempt_to"
        );

        // Save the interrupted context and release the CONTEXT lock.
        let mut guard = CTX_GUARD
            .take()
            .expect("CTX_GUARD not set during preemption");
        guard.save_from_interrupt(ctx);
        drop(guard);

        let thread = this_thread();
        let is_idle = IS_IDLE.load(Ordering::Relaxed);

        thread_exit();

        // can't queue idle
        if !is_idle && requeue {
            LOCAL_WORK_QUEUE.lock().push_back(thread);
        }

        unsafe { go_to_thread(target) }
    }
}

// flowey writes "worst function in mos history" asked to drop the class
fn suspend_impl<T: FnOnce(Arc<Thread>)>(action: T, target: Arc<Thread>) {
    assert!(
        !Arch::irq_is_enabled(),
        "suspend_impl requires irq to be disabled on entry"
    );

    assert!(
        is_on_thread(),
        "suspend_impl() called when the current core is not in a thread context"
    );

    let thread = this_thread();
    let context = CTX_GUARD.take().expect("CTX_GUARD not set");

    unsafe {
        Arch::save_context(&(*CTX_SWITCH_STACK).0, context, move || {
            thread_exit();
            action(thread);
            debug_assert!(
                !Arch::irq_is_enabled(),
                "action() re-enabled IRQ - this should never happen"
            );
            go_to_thread(target);
        })
    }
}

/// Preempt to the idle thread, for general purpose rescheduling.
/// # Safety
/// Can only be called from IRQ
pub unsafe fn preempt_to_idle(ctx: &InterruptContext) {
    unsafe { preempt_to(ctx, IDLE.get().unwrap().clone(), true) }
}

/// Preempt to the idle thread, for general purpose rescheduling.
/// # Safety
/// Can only be called from IRQ
pub unsafe fn block_to_idle(ctx: &InterruptContext) {
    unsafe { preempt_to(ctx, IDLE.get().unwrap().clone(), false) }
}

#[inline(always)]
pub fn suspend_to_queue<T: MutexLike<ThreadQueue>>(queue: &T) {
    let guard = StateGuard::<Irq>::guard();
    if PINNED_TO_CORE.load(Ordering::Relaxed) {
        CORE_PINNED_TO.store(CORE_ID.get().0, Ordering::Relaxed);
    }
    drop(guard);

    let _guard = StateGuard::<Irq>::preserve();

    let mut queue = queue.lock_no_restore_irq();

    assert!(
        !Arch::irq_is_enabled(),
        "interrupts must either be disabled by the lock, or be disabled on entry"
    );

    suspend_impl(
        move |t| {
            queue.push_back(t);
            drop(queue); // okay to just drop the queue here, because we locked it with
            // lock_no_restore_irq, so there's no chance of irqs randomly being
            // restored here
            //
            // the _guard is used to actually restore irq state when needed
        },
        IDLE.get().unwrap().clone(),
    );
}

/// Like `suspend_to_queue`, but only actually suspends if `condition()` returns true
/// after the queue lock is held. This closes the TOCTOU window where the lock owner
/// releases the lock and finds the queue empty, then this thread enqueues itself and
/// sleeps forever.
#[inline(always)]
pub fn suspend_to_queue_if<T, F>(queue: &T, condition: F)
where
    T: MutexLike<ThreadQueue>,
    F: FnOnce() -> bool,
{
    let guard = StateGuard::<Irq>::guard();
    if PINNED_TO_CORE.load(Ordering::Relaxed) {
        CORE_PINNED_TO.store(CORE_ID.get().0, Ordering::Relaxed);
    }
    drop(guard);

    let _guard = StateGuard::<Irq>::preserve();

    let mut queue = queue.lock_no_restore_irq();

    assert!(
        !Arch::irq_is_enabled(),
        "interrupts must either be disabled by the lock, or be disabled on entry"
    );

    // Re-check the condition while holding the queue lock. If the lock owner
    // already released and is about to wake waiters, we'll see the lock as free
    // here and bail out instead of sleeping forever.
    if !condition() {
        return;
    }

    suspend_impl(
        move |t| {
            queue.push_back(t);
            drop(queue);
        },
        IDLE.get().unwrap().clone(),
    );
}

#[inline(always)]
// Queue must already be locked.
// adds the current thread to the queue, unlocks it, then switches to idle
// may combine with suspend_to_queue later
pub fn suspend_to_locked_queue<G>(mut guard: G)
where
    G: DerefMut<Target = ThreadQueue>,
{
    suspend_impl(
        move |t| {
            guard.push_back(t);
            drop(guard);
        },
        IDLE.get().unwrap().clone(),
    );
}

#[inline(always)]
pub fn suspend_to_thread(thread: Arc<Thread>) {
    let _guard = StateGuard::<Irq>::guard();
    suspend_impl(drop, thread);
}

#[inline(always)]
pub fn yield_thread() {
    // TODO use the schedule_thread() function
    if PINNED_TO_CORE.load(Ordering::Relaxed) {
        suspend_to_queue(&*LOCAL_WORK_QUEUE);
    } else {
        suspend_to_queue(&GLOBAL_WORK_QUEUE);
    }
}

pub fn schedule_thread(task: Arc<Thread>) {
    if PINNED_TO_CORE.read_for(&task).load(Ordering::Relaxed) {
        let core = CoreId(CORE_PINNED_TO.read_for(&task).load(Ordering::Relaxed));
        LOCAL_WORK_QUEUE.read_for(core).lock().push_back(task);
    } else {
        LOCAL_WORK_QUEUE.lock().push_back(task);
    }
}

pub fn make_thread<T: FnOnce() + Send + 'static>(task: T) -> Arc<Thread> {
    unsafe extern "C" fn thread_entry<T: FnOnce()>(task: *mut T) -> ! {
        {
            let task = unsafe { Box::from_raw(task) };
            task();
        }

        suspend_to_thread(IDLE.get().unwrap().clone());
        panic!("unreachable")
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
    Arch::wake_other_cores();
}

pub fn make_user_thread(process: &Arc<Process>, pc: usize, sp: usize) -> Arc<Thread> {
    let thread = Thread::new();
    thread.process.call_once(|| Arc::clone(process));

    {
        let mut ctx = CONTEXT.read_for(&thread).lock();
        *ctx = Context::new_uthread(pc as u64, sp as u64);
    }

    CAN_YIELD.read_for(&thread).store(true, Ordering::Relaxed);
    thread.clone()
}

pub fn spawn_user_thread(process: &Arc<Process>, pc: usize, sp: usize) {
    let thread = make_user_thread(process, pc, sp);
    GLOBAL_WORK_QUEUE.lock().push_back(thread);
    Arch::wake_other_cores();
}
