extern crate alloc;

use core::{
    arch::naked_asm,
    cell::{Cell, OnceCell},
    ffi::c_void,
    mem::forget,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

use alloc::boxed::Box;
use alloc::sync::Arc;
use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use spin::{Mutex, MutexGuard, Once};

use crate::{
    arch::{
        Context, IrqState, get_thread_local_pointer, save_context, set_thread_local_pointer,
        sleep_core,
    },
    local_storage::{LocalStorage, LocalStorageHandler, impl_local_storage},
    mp::{CORE_ID, MP_STAGE, MPStage, core_local},
    print::kprintln,
    sync::IntMutex,
};

pub struct Thread {
    pub link: LinkedListAtomicLink,
    pub tls: Box<[u8]>,
    pub tls_addr: u64, // aliased to tls
}

impl Thread {
    pub fn new() -> Arc<Thread> {
        let tls = ThreadLocalStorageHandler::create();
        let tls_addr = Box::as_ptr(&tls).as_ptr() as u64;

        let handle = Arc::new(Thread {
            link: LinkedListAtomicLink::new(),
            tls,
            tls_addr,
        });

        THIS_THREAD.read_for(&handle).call_once(|| handle.clone());

        handle
    }

    pub fn is_same_thread(lhs: &Arc<Thread>, rhs: &Arc<Thread>) -> bool {
        Arc::as_ptr(lhs) == Arc::as_ptr(rhs)
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
        unsafe { get_thread_local_pointer() }
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

core_local! {
    pub IDLE: OnceCell<Arc<Thread>> = OnceCell::new();
    IS_ON_THREAD: AtomicBool = AtomicBool::new(false);
}

// TODO: alignment here is ABI specific, this needs to be moved into src/arch
#[repr(align(16))]
struct Stack([u8; 32 * 4096]);

thread_local! {
    pub THIS_THREAD: Once<Arc<Thread>> = Once::new();
    pub CONTEXT: Mutex<Context> = Mutex::new(Default::default());
    pub CAN_YIELD: AtomicBool = AtomicBool::new(false);
    STACK: Stack = Stack([0; _]);
    CTX_GUARD: Cell<Option<MutexGuard<'static, Context>>> = Cell::new(None);
}

static GLOBAL_WORK_QUEUE: Once<IntMutex<ThreadQueue>> = Once::new();

pub fn init_threading() {
    GLOBAL_WORK_QUEUE.call_once(|| IntMutex::new(new_thread_queue()));
}

fn thread_enter(tls_addr: *const u64) {
    unsafe { set_thread_local_pointer(tls_addr) };
    IS_ON_THREAD.store(true, Ordering::Relaxed);
}

fn thread_exit() {
    IS_ON_THREAD.store(false, Ordering::Relaxed);
    unsafe { set_thread_local_pointer(ptr::null()) };
}

pub fn is_on_thread() -> bool {
    IS_ON_THREAD.load(Ordering::Relaxed)
}

unsafe fn go_to_thread(thread: &Thread) -> ! {
    assert!(!is_on_thread());

    thread_enter(&thread.tls_addr);

    let ctx = CONTEXT.lock();
    let state: Context = *ctx;
    CTX_GUARD.set(Some(ctx));

    state.jump_to();
}

pub fn set_up_idle() {
    let Ok(_) = IDLE.set(Thread::new()) else {
        panic!("expected core-local idle to be not init");
    };

    thread_enter(&IDLE.get().unwrap().tls_addr);
    let mut ctx = CONTEXT.lock();
    ctx.setup_kthread_context();
    CTX_GUARD.set(Some(ctx));
}

pub fn poll_tasks() -> ! {
    assert!(
        Thread::is_same_thread(THIS_THREAD.get().unwrap(), IDLE.get().unwrap()),
        "poll_tasks may only be called from idle"
    );

    loop {
        assert!(
            !IrqState::save().is_masked(),
            "irq must be enabled when polling tasks"
        );

        let mut queue = GLOBAL_WORK_QUEUE.get().unwrap().lock();

        let Some(x) = queue.pop_front() else {
            drop(queue);
            // kprintln!("core {}: sleeping because no tasks", CORE_ID.get());
            // sleep_core();
            continue;
        };

        yield_thread_with_action_to(
            |_| {
                drop(queue);
            },
            &x,
        );
    }
}

pub fn can_yield() -> bool {
    // we can only yield if we are in a thread context
    // and the current thread can yield (for instance, idle cannot yield)
    MP_STAGE.load(Ordering::Relaxed) == MPStage::MPPreempt
        && is_on_thread()
        && CAN_YIELD.load(Ordering::Relaxed)
}

// flowey writes "worst function in mos history" asked to drop the class
pub fn yield_thread_with_action_to<T: FnOnce(Arc<Thread>)>(pre_suspend_action: T, target: &Thread) {
    assert!(IrqState::save().is_masked());
    assert!(
        is_on_thread(),
        "yield_thread_with_action_to() called when the current core is not in a thread context"
    );

    let thread = THIS_THREAD.get().expect("THIS_THREAD not set").clone();
    let mut context = CTX_GUARD.take().expect("CTX_GUARD not set");

    unsafe {
        // THERE'S NO WAY THIS IS SAFE RIGHT
        if save_context(&mut context) {
            // must use forget to prevent double free
            // TODO: this is really unsafe
            forget(thread);
            forget(context);
            forget(pre_suspend_action);

            return;
        }
    }

    thread_exit();

    pre_suspend_action(thread);
    drop(context);

    unsafe { go_to_thread(target) };
}

#[inline(always)]
pub fn yield_thread() {
    yield_thread_with_action_to(|_| {}, IDLE.get().unwrap());
}

#[inline(always)]
pub fn yield_thread_to(target: &Thread) {
    yield_thread_with_action_to(|_| {}, target);
}

#[inline(always)]
pub fn yield_thread_with_action<T: FnOnce(Arc<Thread>)>(pre_suspend_action: T) {
    yield_thread_with_action_to(pre_suspend_action, IDLE.get().unwrap());
}

pub fn spawn_thread<T: FnOnce()>(task: T) {
    unsafe extern "C" fn thread_entry<T: FnOnce()>(task: *mut T) -> ! {
        {
            let task = unsafe { Box::from_raw(task) };
            task();
        }

        yield_thread_with_action(|_| {
            todo!("implement thread cleanup");
        });

        panic!("unreachable")
    }

    // need to push zero because of SysV abi:
    // "the stack frame needs to be 16-byte aligned before a call, and 8 byte misaligned after the
    // call"
    #[unsafe(naked)]
    unsafe extern "C" fn thread_entry0<T: FnOnce()>(task: *mut T) -> ! {
        naked_asm!(
            "pushq $0",
            "jmp {0}",
            sym thread_entry::<T>,
            options(att_syntax)
        )
    }

    let thread = Thread::new();

    {
        let mut ctx = CONTEXT.read_for(&thread).lock();
        let task = Box::into_raw(Box::new(task));
        ctx.setup_for_call(&(*STACK).0, thread_entry0, task);
    }

    GLOBAL_WORK_QUEUE.get().unwrap().lock().push_back(thread);
}
