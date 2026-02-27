use crate::{
    // arch::x86_64::cpuid::Features,
    kernel_main,
    mp::{core_local, get_cpu_local_pointer_for, init_cpu_local_table, CoreId, CORE_ID},
    print::kprintln,
};
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};
use limine::{mp::Cpu, request::MpRequest};

#[used]
#[unsafe(link_section = ".limine_requests")]
static MP_REQUEST: MpRequest = MpRequest::new();

core_local! {
    pub MPDIR_ID: AtomicU64 = AtomicU64::new(0);
}

pub fn core_count() -> usize {
    MP_REQUEST.get_response().unwrap().cpus().len()
}

fn enable_advsimd() {
    // CPACR_EL1.FPEN[21:20] = 0b11: do not trap FP/AdvSIMD at EL0/EL1.
    unsafe {
        asm!(
            "mrs x0, cpacr_el1",
            "orr x0, x0, #(3 << 20)",
            "msr cpacr_el1, x0",
            "isb",
            out("x0") _,
            options(nomem, nostack, preserves_flags),
        );
    }
}

fn init_cpu_local_ptr(core_id: CoreId) {
    let ptr = get_cpu_local_pointer_for(core_id);
    unsafe {
        asm!(
            "msr tpidr_el1, {}",
            in(reg) ptr,
            options(nomem, nostack, preserves_flags),
        )
    };
}

pub fn get_cpu_local_pointer() -> u64 {
    let mut slot: u64;

    unsafe {
        asm!(
            "mrs {}, tpidr_el1",
            lateout(reg) slot,
            options(nomem, nostack, preserves_flags),
        );
    }

    unsafe { *(slot as *const u64) }
}

pub unsafe fn set_thread_local_pointer(base: *const u64) {
    unsafe {
        asm!(
            "msr tpidr_el0, {}",
            in(reg) base as u64,
            options(nomem, nostack, preserves_flags),
        )
    };
}

pub unsafe fn get_thread_local_pointer() -> u64 {
    let mut slot: u64;

    unsafe {
        asm!(
            "mrs {} , tpidr_el0",
            lateout(reg) slot,
            options(nomem, nostack, preserves_flags),
        );
    }

    unsafe { *(slot as *const u64) }
}

pub fn initialize_mp() -> ! {
    let response = MP_REQUEST.get_response().expect("mp response not received");

    let n_cores = response.cpus().len();
    kprintln!("aarch64::initialize_mp(): bootstrapping {} cores", n_cores);

    init_cpu_local_table(n_cores);

    let mut core_id: u64 = 1;
    let bsp_id = response.bsp_mpidr();

    let mut core_self = None;

    for cpu in response.cpus() {
        if bsp_id != cpu.mpidr {
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
    enable_advsimd();

    let id = CoreId(cpu.extra.load(Ordering::SeqCst) as usize);
    init_cpu_local_ptr(id);
    CORE_ID.replace(id);
    MPDIR_ID.store(cpu.mpidr as u64, Ordering::Relaxed);
    kprintln!(
        "done init core {}, CLS base={:x}, TPIDR_EL1={:x}",
        id,
        get_cpu_local_pointer(),
        get_cpu_local_pointer_for(id)
    );

    // TODO: handle interrupts

    kernel_main()
}
