use core::arch::asm;
use core::arch::naked_asm;
use core::fmt;
use core::fmt::Display;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering;

use limine::mp::Cpu;
use limine::request::MpRequest;
use spin::Once;
use x86::bits64::registers::rbp;
use x86::bits64::rflags;
use x86::bits64::rflags::RFlags;
use x86::cpuid::CpuId;
use x86::msr::IA32_GS_BASE;
use x86::msr::wrmsr;

use crate::percore::CORE_ID;
use crate::percore::CoreId;
use crate::percore::core_local;
use crate::percore::get_cpu_local_pointer_for;
use crate::percore::init_cpu_local_table;
use crate::print::ANSIFormatter;
use crate::print::Color;
use crate::print::kprint;
use crate::print::kprintln;

pub fn halt() -> ! {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        asm!("cli");
        loop {
            asm!("hlt");
        }
    }
}

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    naked_asm!(
        "mov rax, rdi",
        "mov rcx, rdx",
        "shr rcx, 3",
        "rep movsq",
        "mov rcx, rdx",
        "and rcx, 0x7",
        "rep movsb",
        "ret",
    )
}

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dest: *mut u8, byte: i32, len: usize) -> *mut u8 {
    naked_asm!(
        "mov r11, rdi",
        "mov rcx, rdx",
        "movzx rax, sil",
        "mov r10, 0x0101010101010101",
        "mul r10",
        "mov rdx, rcx",
        "shr rcx, 3",
        "rep stosq",
        "mov rcx, rdx",
        "and rcx, 0x7",
        "rep stosb",
        "mov rax, r11",
        "ret",
    )
}

#[inline(always)]
fn disable_interrupts() {
    unsafe {
        asm!("cli");
    }
}

#[inline(always)]
fn enable_interrupts() {
    unsafe {
        asm!("sti");
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct IrqState(bool);

impl IrqState {
    #[inline(always)]
    pub fn save() -> IrqState {
        IrqState(rflags::read().contains(RFlags::FLAGS_IF))
    }

    #[inline(always)]
    pub fn restore(self) {
        if self.0 {
            enable_interrupts();
        } else {
            disable_interrupts();
        }
    }
}

#[inline(always)]
pub fn irq_disable() {
    disable_interrupts();
}

#[derive(Clone, Copy)]
pub struct UnwindContext {
    ptr: *const u64,
}

impl UnwindContext {
    #[inline(always)]
    pub unsafe fn get() -> UnwindContext {
        UnwindContext {
            ptr: rbp() as *const u64,
        }
    }

    pub unsafe fn valid(&self) -> bool {
        (unsafe { self.return_address() }) != 0
    }

    pub unsafe fn return_address(&self) -> u64 {
        unsafe { self.ptr.wrapping_add(1).read() }
    }

    pub unsafe fn next(&self) -> UnwindContext {
        UnwindContext {
            ptr: unsafe { self.ptr.read() } as *const u64,
        }
    }
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

#[used]
#[unsafe(link_section = ".limine_requests")]
static MP_REQUEST: MpRequest = MpRequest::new();

fn init_cpu_local_ptr(core_id: CoreId) {
    // Gheith uses fsgsbase instructions here, but the MSR is older
    let ptr = get_cpu_local_pointer_for(core_id);
    unsafe { wrmsr(IA32_GS_BASE, ptr) };
}

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

static CPU_ID_PRINT: Once<()> = Once::new();

struct Features<'a>(&'a CpuId);

impl<'a> Display for Features<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn format(flag: bool) -> ANSIFormatter<'static, &'static str> {
            if flag {
                Color::RED.format(&"not present")
            } else {
                Color::GREEN.format(&"present")
            }
        }

        if let Some(features) = self.0.get_feature_info() {
            let _ = writeln!(f, "features:");
            let _ = writeln!(f, "avx = {}", format(features.has_avx()));
            let _ = writeln!(f, "sse4.2 = {}", format(features.has_sse42()));
            let _ = writeln!(f, "sse4.1 = {}", format(features.has_sse41()));
            let _ = writeln!(f, "ssse3 = {}", format(features.has_ssse3()));
            let _ = writeln!(f, "sse3 = {}", format(features.has_sse3()));
            let _ = writeln!(f, "xsave = {}", format(features.has_xsave()));
            let _ = writeln!(f, "oxsave = {}", format(features.has_oxsave()));
            let _ = writeln!(
                f,
                "monitor/mwait = {}",
                format(features.has_monitor_mwait())
            );
            let _ = writeln!(f, "vmx = {}", format(features.has_vmx()));
        } else {
            let _ = writeln!(f, "{}", Color::RED.format(&"features not detected"));
        }

        if let Some(features) = self.0.get_extended_feature_info() {
            let _ = writeln!(f, "extended features:");
            let _ = writeln!(f, "fsgsbase = {}", format(features.has_fsgsbase()));
        } else {
            let _ = writeln!(
                f,
                "{}",
                Color::RED.format(&"extended features not detected")
            );
        }

        // TODO: svm, rdtscp

        Ok(())
    }
}

unsafe extern "C" fn initialize_core(cpu: &Cpu) -> ! {
    kprintln!("hello from x86::initialize_core");
    let id = CoreId(cpu.extra.load(Ordering::SeqCst) as usize);
    init_cpu_local_ptr(id);
    CORE_ID.replace(id);
    LAPIC_ID.store(cpu.lapic_id, Ordering::Relaxed);
    kprintln!("done init core {}", id);

    let cpu_id = CpuId::new();

    CPU_ID_PRINT.call_once(|| kprint!("{}", Features(&cpu_id)));

    halt()
}

core_local! {
    pub LAPIC_ID: AtomicU32 = AtomicU32::new(0);
}

#[repr(C)]
struct InterruptContext {
    regs: [u64; 14],
    id: u64,
    err: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

#[derive(Debug, Default)]
pub struct GPRegisters {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

pub struct Context {
    pub gp: GPRegisters,
    pub rip: u64,
    pub rflags: RFlags,

    // 16 bit, extended to 64
    pub cs: u64,
    pub ss: u64,
}

