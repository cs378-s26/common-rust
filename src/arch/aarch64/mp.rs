use crate::{
    // arch::x86_64::cpuid::Features,
    mp::{core_local, get_cpu_local_pointer_for, init_cpu_local_table, CoreId, CORE_ID},
    print::{kprint, kprintln},
};
use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};
use limine::{mp::Cpu, request::MpRequest};

#[used]
#[unsafe(link_section = ".limine_requests")]
static MP_REQUEST: MpRequest = MpRequest::new();

core_local! {
    pub MPDIR_ID: AtomicU32 = AtomicU32::new(0);
    IST: Once<InterruptStackTable> = Once::new();
    GDT: Once<GlobalDescriptorTable> = Once::new();
    IDT: Once<InterruptDescriptorTable> = Once::new();
}

pub fn core_count() -> usize {
    MP_REQUEST.get_response().unwrap().cpus().len()
}

fn init_cpu_local_ptr(core_id: CoreId) {
    let ptr = get_cpu_local_pointer_for(core_id);
    unsafe {
        asm!(
            "msr tpidr_el1, {}",
            in(reg) ptr,
            options(nostack, preserves_flags, pure),
        )
    };
}

pub fn get_cpu_local_pointer() -> u64 {
    let mut val: u64;

    unsafe {
        asm!(
            "mrs {}, tpidr_el1",
            lateout(reg) val,
            options(nostack, preserves_flags, pure, readonly),
        );
    }

    val
}

pub unsafe fn set_thread_local_pointer(base: *const u64) {
    unsafe {
        asm!(
            "msr tpidr_el0, {}",
            in(reg) base as u64,
            options(nostack, preserves_flags, pure),
        )
    };
}

pub unsafe fn get_thread_local_pointer() -> u64 {
    let mut val: u64;

    unsafe {
        asm!(
            "mrs {} , tpidr_el0",
            lateout(reg) val,
            options(nostack, preserves_flags, pure, readonly),
        );
    }

    val
}

pub fn initialize_mp() -> ! {
    let response = MP_REQUEST.get_response().expect("mp response not received");

    let n_cores = response.cpus().len();
    kprintln!("aarch64::initialize_mp(): bootstrapping {} cores", n_cores);

    init_cpu_local_table(n_cores);

    let mut core_id: u64 = 1;
    let bsp_id = response.bsp_mpdir();

    let mut core_self = None;

    for cpu in response.cpus() {
        if bsp_id != cpu.mpdir {
            cpu.extra.store(core_id, Ordering::SeqCst);
            core_id += 1;
            cpu.goto_address.write(initialize_core);
        } else {
            core_self = Some(cpu);
        }
    }

    unsafe { initialize_core(core_self.expect("limine did not give current CPU in MP response")) };
}

unsafe extern "C" fn initialize_core(cpu: &Cpu) -> ! {
    // kprintln!("hello from x86::initialize_core");
    let id = CoreId(cpu.extra.load(Ordering::SeqCst) as usize);
    init_cpu_local_ptr(id);
    CORE_ID.replace(id);
    MPDIR_ID.store(cpu.lapic_id, Ordering::Relaxed);
    kprintln!(
        "done init core {}, CLS base={:x}, TPDIR_EL1={:x}",
        id,
        get_cpu_local_pointer(),
        get_cpu_local_pointer_for(id)
    );

    // let cpu_id = CpuId::new();

    // CPU_ID_PRINT.call_once(|| kprint!("{}", Features(&cpu_id)));

    fn allocate_sp(pages: usize) -> u64 {
        slice_stack_pointer(unsafe { &*Box::into_raw(aligned_slice(pages * 4096, 4096)) })
    }

    // TODO: handle interrupts

    kernel_main()
}
