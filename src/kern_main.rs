
use core::arch::asm;
use core::sync::atomic::Ordering;

// For coroutines.
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use limine::BaseRevision;
use limine::firmware_type::FirmwareType;
use limine::request::{
    BootloaderInfoRequest, FirmwareTypeRequest, RequestsEndMarker, RequestsStartMarker,
};
use spin::{Barrier, Once};
use talc::Span;
use x86::time::rdtsc;

use crate::arch::{core_count, initialize_mp, irq_enable};
use crate::coroutine::{init_coroutine_executor, init_coroutine_queue, spawn_coroutine};
use crate::cmdline::{get_cmdline_error, get_cmdline_text, parse_kernel_cmdline};
use crate::heap::init_malloc;
use crate::mp::{CORE_ID, MP_STAGE, MPStage};
use crate::print::{init_tty, kprintln};
use crate::thread::{Thread, init_threading, poll_tasks, set_up_idle, spawn_thread, yield_thread};

