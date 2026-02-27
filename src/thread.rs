extern crate alloc;

use core::{
    cell::{Cell, LazyCell, OnceCell, RefCell, RefMut},
    ffi::c_void,
    pin::Pin,
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(target_arch = "x86_64")]
use core::arch::naked_asm;

use alloc::boxed::Box;
use alloc::sync::Arc;
use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use spin::{Mutex, MutexGuard, Once};

use crate::{
    arch::{
        Arch, ArchTrait, Context, ContextTrait, InterruptContext, IrqState, IrqStateTrait,
        irq_is_enabled, sleep_core,
    },
    local_storage::{LocalStorage, LocalStorageHandler, impl_local_storage},
    mp::{CORE_ID, MP_STAGE, MPStage, core_local},
    print::kprintln,
    sync::{IntMutex, MutexLike},
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

        THIS_THREAD.read_for(&handle).call_once(|| handle.clone());

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
        Arch::get_thread_local_pointer()
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

pub fn new_thread_queue() -> ThreadQueue {
    ThreadQueue::new(ThreadQueueAdapter::new())
}

// thread scheduling and management

// TODO: alignment here is ABI specific, this needs to be moved into src/arch
#[repr(align(16))]
struct Stack([u8; 4 * 4096]);

core_local! {
    pub IDLE: OnceCell<Arc<Thread>> = OnceCell::new();
    CURRENT_THREAD: Cell<Option<Arc<Thread>>> = Cell::new(None);
    CTX_SWITCH_STACK: Stack = Stack([0; _]);
    LOCAL_WORK_QUEUE: LazyCell<RefCell<ThreadQueue>> = LazyCell::new(|| RefCell::new(new_thread_queue()));
}

thread_local! {
    pub THIS_THREAD: Once<Arc<Thread>> = Once::new();
    pub CONTEXT: Mutex<Context> = Mutex::new(Default::default());
    pub CAN_YIELD: AtomicBool = AtomicBool::new(false);
    TID: AtomicU64 = AtomicU64::new(0);
    STACK: Stack = Stack([0; _]);
    CTX_GUARD: Cell<Option<MutexGuard<'static, Context>>> = Cell::new(None);
}

static CURR_TID: AtomicU64 = AtomicU64::new(1);

static GLOBAL_WORK_QUEUE: Once<IntMutex<ThreadQueue>> = Once::new();

pub fn init_threading() {
    GLOBAL_WORK_QUEUE.call_once(|| IntMutex::new(new_thread_queue()));
}

pub fn local_work_queue() -> RefMut<'static, ThreadQueue> {
    LOCAL_WORK_QUEUE.borrow_mut()
}

fn thread_enter(thread: Arc<Thread>) {
    Arch::set_thread_local_pointer(&thread.tls_addr);
    CURRENT_THREAD.set(Some(thread));
}

fn thread_exit() {
    CURRENT_THREAD.set(None);
    Arch::set_thread_local_pointer(ptr::null());
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
        Thread::is_same_thread(THIS_THREAD.get().unwrap(), IDLE.get().unwrap()),
        "poll_tasks may only be called from idle"
    );

    loop {
        loop {
            let Some(thread) = local_work_queue().pop_front() else {
                break;
            };

            GLOBAL_WORK_QUEUE.get().unwrap().lock().push_back(thread);
        }

        let thread = {
            let mut lock = GLOBAL_WORK_QUEUE.get().unwrap().lock();
            let task = lock.pop_front();
            drop(lock);
            task
        };

        let Some(thread) = thread else {
            Arch::set_irq_enabled(true); // unmask interrupts
            kprintln!("core {}: sleeping because no tasks", CORE_ID.get());
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
    // interrupt handler or critical section — don't yield there).
    irq_is_enabled()
        && MP_STAGE.load(Ordering::Relaxed) == MPStage::MPPreempt
        && is_on_thread()
        && CAN_YIELD.load(Ordering::Relaxed)
}

/// Called from the timer interrupt handler to preemptively switch the current thread.
/// Interrupts are disabled on entry (x86 clears IF on interrupt). If the current thread
/// is not preemptable this returns immediately; otherwise it never returns — it saves the
/// interrupted context, pushes the thread back onto the global queue, and switches to idle.
///
/// `rbp` is the original rbp value at interrupt time, passed separately because
/// irq_handler_t0 does not include it in the InterruptContext register array.
pub unsafe fn preempt_from_interrupt(ctx: &InterruptContext, rbp: u64) {
    if !can_yield_for_preempt() {
        return;
    }
    unsafe { do_preempt(ctx, rbp) }
}

/// Like `can_yield` but skips the IRQ-enabled check — we call this from the timer ISR
/// where IRQs are already disabled by the CPU, but we still want to preempt.
fn can_yield_for_preempt() -> bool {
    MP_STAGE.load(Ordering::Relaxed) == MPStage::MPPreempt
        && is_on_thread()
        && CAN_YIELD.load(Ordering::Relaxed)
}

unsafe fn do_preempt(ctx: &InterruptContext, rbp: u64) -> ! {
    use x86::bits64::rflags::RFlags;

    // --- 1. Save the interrupted thread's full context ---
    // regs[] layout (low to high in memory = last-pushed to first-pushed):
    // [0]=r15 [1]=r14 [2]=r13 [3]=r12 [4]=r11 [5]=r10 [6]=r9 [7]=r8
    // [8]=rdi [9]=rsi [10]=rbx [11]=rdx [12]=rcx [13]=rax
    let mut guard = CTX_GUARD
        .take()
        .expect("CTX_GUARD not set during preemption");
    guard.gp.r15 = ctx.regs[0];
    guard.gp.r14 = ctx.regs[1];
    guard.gp.r13 = ctx.regs[2];
    guard.gp.r12 = ctx.regs[3];
    guard.gp.r11 = ctx.regs[4];
    guard.gp.r10 = ctx.regs[5];
    guard.gp.r9 = ctx.regs[6];
    guard.gp.r8 = ctx.regs[7];
    guard.gp.rdi = ctx.regs[8];
    guard.gp.rsi = ctx.regs[9];
    guard.gp.rbx = ctx.regs[10];
    guard.gp.rdx = ctx.regs[11];
    guard.gp.rcx = ctx.regs[12];
    guard.gp.rax = ctx.regs[13];
    guard.gp.rbp = rbp;
    guard.gp.rsp = ctx.rsp; // hardware-saved RSP (stack pointer before interrupt)
    guard.rip = ctx.rip;
    guard.rflags = RFlags::from_bits_truncate(ctx.rflags);
    // cs/ss are already correct from setup_kthread_context — don't overwrite

    // Release the CONTEXT lock. The thread's context is now safe to resume on any core.
    drop(guard);

    // --- 2. Stash the thread Arc for re-queuing after stack switch ---
    // We must not push to GLOBAL_WORK_QUEUE yet: we're still on the thread's stack.
    // Another core could pick it up and clobber this stack while we're using it.
    // Pass it as a raw pointer through the assembly stack switch.
    let thread = THIS_THREAD.get().expect("THIS_THREAD not set").clone();
    let thread_raw = Arc::into_raw(thread) as u64; // leaks one refcount, restored in go_to_idle_inner

    // --- 3. Leave thread context (clears CURRENT_THREAD and FS base) ---
    thread_exit();

    // --- 4. Switch to CTX_SWITCH_STACK, push thread to queue, go to idle ---
    // After this we are no longer on the thread's stack, so it is safe to queue.
    // CTX_SWITCH_STACK is core-local (GS-based), still accessible after thread_exit.
    let stack: &[u8] = &(*CTX_SWITCH_STACK).0;
    let stack_top = stack.as_ptr_range().end as u64;
    unsafe { switch_stack_and_idle(stack_top, thread_raw) }
}

/// Naked trampoline: switches RSP to `stack_top`, then tail-calls `go_to_idle_inner(thread_raw)`.
#[unsafe(naked)]
unsafe extern "C" fn switch_stack_and_idle(stack_top: u64, thread_raw: u64) -> ! {
    naked_asm!(
        "movq %rdi, %rsp",  // switch to CTX_SWITCH_STACK (stack_top in rdi)
        "movq %rsi, %rdi",  // thread_raw becomes 1st arg for go_to_idle_inner
        "jmp {0}",
        sym go_to_idle_inner,
        options(att_syntax)
    )
}

/// Naked trampoline: switches RSP to `stack_top`, then calls `go_to_idle_direct()`.
/// Used when a thread finishes normally (no thread to re-queue).
#[unsafe(naked)]
unsafe extern "C" fn switch_stack_to_idle(stack_top: u64) -> ! {
    naked_asm!(
        "movq %rdi, %rsp",  // switch to CTX_SWITCH_STACK
        "jmp {0}",
        sym go_to_idle_direct,
        options(att_syntax)
    )
}

/// Called on CTX_SWITCH_STACK after a preemptive context switch. Re-queues the preempted
/// thread and jumps to idle to pick up the next runnable thread.
extern "C" fn go_to_idle_inner(thread_raw: u64) -> ! {
    // Reconstruct the Arc (balances the into_raw leak from do_preempt).
    let thread = unsafe { Arc::from_raw(thread_raw as *const Thread) };
    // Now safe to queue: we are on CTX_SWITCH_STACK, not the thread's stack.
    GLOBAL_WORK_QUEUE.get().unwrap().lock().push_back(thread);
    unsafe { go_to_thread(IDLE.get().unwrap().clone()) }
}

/// Called on CTX_SWITCH_STACK when a thread exits normally (no re-queuing needed).
extern "C" fn go_to_idle_direct() -> ! {
    unsafe { go_to_thread(IDLE.get().unwrap().clone()) }
}

// flowey writes "worst function in mos history" asked to drop the class
fn suspend_impl<T: FnOnce(Arc<Thread>)>(action: T, target: Arc<Thread>) {
    let irq_state = IrqState::save();
    Arch::set_irq_enabled(false);

    assert!(
        is_on_thread(),
        "yield_thread_with_action_to() called when the current core is not in a thread context"
    );

    let thread = THIS_THREAD.get().expect("THIS_THREAD not set").clone();
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
    let queue = GLOBAL_WORK_QUEUE.get().unwrap();
    suspend_to_queue(queue);
}

pub fn spawn_thread<T: FnOnce() + Send + 'static>(task: T) {
    unsafe extern "C" fn thread_entry<T: FnOnce()>(task: *mut T) -> ! {
        {
            let task = unsafe { Box::from_raw(task) };
            task();
        }

        // Task finished — exit thread and return to idle on a safe stack.
        // Drop the CTX_GUARD (releases CONTEXT lock) then leave thread context.
        CAN_YIELD.store(false, Ordering::Relaxed);
        CTX_GUARD.take();
        thread_exit();

        // Switch to CTX_SWITCH_STACK and jump to idle — we must not use our own
        // stack after thread_exit() since another core could reuse it.
        let stack: &[u8] = &(*CTX_SWITCH_STACK).0;
        let stack_top = stack.as_ptr_range().end as u64;
        unsafe { switch_stack_to_idle(stack_top) }
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

    GLOBAL_WORK_QUEUE.get().unwrap().lock().push_back(thread);
}
