extern crate alloc;

use core::ops::DerefMut;
use core::{
    arch::naked_asm,
    cell::{Cell, LazyCell, OnceCell, RefCell, RefMut},
    ffi::c_void,
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use alloc::boxed::Box;
use alloc::sync::Arc;
use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use spin::{Mutex, MutexGuard, Once};

use crate::{
    arch::{
        Context, IrqState, get_thread_local_pointer, irq_disable, irq_enable, save_context,
        set_thread_local_pointer, sleep_core,
    },
    local_storage::{LocalStorage, LocalStorageHandler, impl_local_storage},
    mp::{CORE_ID, MP_STAGE, MPStage, core_local},
    print::kprintln,
    sync::{IntMutex, MutexLike},
};

pub struct Thread {
    pub link: LinkedListAtomicLink,
    #[allow(unused)]
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
    unsafe { set_thread_local_pointer(&thread.tls_addr) };
    CURRENT_THREAD.set(Some(thread));
}

fn thread_exit() {
    CURRENT_THREAD.set(None);
    unsafe { set_thread_local_pointer(ptr::null()) };
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
            irq_enable(); // unmask interrupts
            kprintln!("core {}: sleeping because no tasks", CORE_ID.get());
            sleep_core();
            continue;
        };

        suspend_to_thread(thread);
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
fn suspend_impl<T: FnOnce(Arc<Thread>)>(action: T, target: Arc<Thread>) {
    let irq_state = IrqState::save();
    irq_disable();

    assert!(
        is_on_thread(),
        "yield_thread_with_action_to() called when the current core is not in a thread context"
    );

    let thread = THIS_THREAD.get().expect("THIS_THREAD not set").clone();
    let context = CTX_GUARD.take().expect("CTX_GUARD not set");

    unsafe {
        save_context(&(*CTX_SWITCH_STACK).0, context, move || {
            thread_exit();
            action(thread);
            go_to_thread(target);
        })
    };

    irq_state.restore();
}

#[inline(always)]
// Queue must already be locked.
// adds the current thread to the queue, unlocks it, then switches to idle
// may combine with suspend to queue later
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
pub fn suspend_to_queue<T: MutexLike<ThreadQueue>>(queue: &T) {
    // we need to lock *before* suspend_impl, because interrupts are blocked there, and we
    // shouldn't deal with that
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

        todo!()
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
        ctx.setup_for_call(&STACK.read_for(&thread).0, thread_entry0, task);
    }

    CAN_YIELD.read_for(&thread).store(true, Ordering::Relaxed);

    GLOBAL_WORK_QUEUE.get().unwrap().lock().push_back(thread);
}
