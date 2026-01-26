use core::alloc::Layout;
use core::arch::asm;

use core::sync::atomic::{AtomicU32, Ordering};

use alloc::boxed::Box;
use limine::{mp::Cpu, request::MpRequest};
use spin::Once;
use x86::msr::IA32_FS_BASE;
use x86::{
    cpuid::CpuId,
    msr::{IA32_GS_BASE, wrmsr},
};

use crate::arch::x86_64::tables::{
    GlobalDescriptorTable, InterruptDescriptorTable, InterruptStackTable,
};
use crate::heap::{aligned_slice, slice_stack_pointer};
use crate::kernel_main;
use crate::{
    arch::x86_64::cpuid::Features,
    mp::{CORE_ID, CoreId, core_local, get_cpu_local_pointer_for, init_cpu_local_table},
    print::{kprint, kprintln},
};

#[used]
#[unsafe(link_section = ".limine_requests")]
static MP_REQUEST: MpRequest = MpRequest::new();

static CPU_ID_PRINT: Once<()> = Once::new();

core_local! {
    pub LAPIC_ID: AtomicU32 = AtomicU32::new(0);
    IST: Once<InterruptStackTable> = Once::new();
    GDT: Once<GlobalDescriptorTable> = Once::new();
    IDT: Once<InterruptDescriptorTable> = Once::new();
}

// Gheith uses FSGSBASE instructions here, but the MSR is older and doesn't need to be enabled
fn init_cpu_local_ptr(core_id: CoreId) {
    let ptr = get_cpu_local_pointer_for(core_id);
    unsafe { wrmsr(IA32_GS_BASE, ptr) };
}

pub fn get_cpu_local_pointer() -> u64 {
    let mut val: u64;

    unsafe {
        asm!(
            "movq %gs:0, {}",
            lateout(reg) val,
            options(nostack, preserves_flags, pure, readonly, att_syntax),
        );
    }

    val
}

pub unsafe fn set_thread_local_pointer(base: *const u64) {
    unsafe { wrmsr(IA32_FS_BASE, base as u64) };
}

pub unsafe fn get_thread_local_pointer() -> u64 {
    let mut val: u64;

    unsafe {
        asm!(
            "movq %fs:0, {}",
            lateout(reg) val,
            options(nostack, preserves_flags, pure, readonly, att_syntax),
        );
    }

    val
}

pub fn core_count() -> usize {
    MP_REQUEST.get_response().unwrap().cpus().len()
}

// initialization routines

pub fn initialize_mp() -> ! {
    let response = MP_REQUEST.get_response().expect("mp response not received");

    let n_cores = response.cpus().len();
    kprintln!("x86::initialize_mp(): bootstrapping {} cores", n_cores);

    init_cpu_local_table(n_cores);

    let mut core_id: u64 = 1;
    let bsp_id = response.bsp_lapic_id();

    let mut core_self = None;

    for cpu in response.cpus() {
        if bsp_id != cpu.lapic_id {
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
    LAPIC_ID.store(cpu.lapic_id, Ordering::Relaxed);
    kprintln!(
        "done init core {}, CLS base={:x}",
        id,
        get_cpu_local_pointer()
    );

    let cpu_id = CpuId::new();

    CPU_ID_PRINT.call_once(|| kprint!("{}", Features(&cpu_id)));

    fn allocate_sp(pages: usize) -> u64 {
        return slice_stack_pointer(unsafe { &*Box::into_raw(aligned_slice(pages * 4096, 4096)) });
    }

    let ist = IST.call_once(|| InterruptStackTable {
        reserved0: 0,
        rsp0: 0,
        rsp1: 0,
        rsp2: 0,
        reserved1: 0,
        ist1: allocate_sp(32),
        ist2: allocate_sp(32),
        ist3: allocate_sp(32),
        ist4: allocate_sp(32),
        ist5: allocate_sp(32),
        ist6: allocate_sp(32),
        ist7: allocate_sp(32),
        reserved2: 0,
        reserved3: 0,
        io_bp: 0,
    });

    let gdt = GDT.call_once(|| GlobalDescriptorTable::new(ist));
    let idt = IDT.call_once(InterruptDescriptorTable::new);

    unsafe { gdt.load() };
    unsafe { idt.load() };

    // we need to re-load the core local, becase the FS/GSBASE registers are really just references
    // to the "cached" segment base registers, which gets reset on descriptor reloads
    init_cpu_local_ptr(id);

    kernel_main();
}
