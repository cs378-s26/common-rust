use core::arch::naked_asm;

use x86::{Ring, bits64::rflags::RFlags, segmentation::SegmentSelector};

use crate::arch::x86_64::{slice_stack_pointer, tables::GlobalDescriptorTable};

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
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

#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub gp: GPRegisters,
    pub rip: u64,
    pub rflags: RFlags,

    // 16 bit, extended to 64
    pub cs: u64,
    pub ss: u64,
}

impl const Default for Context {
    fn default() -> Self {
        Self {
            gp: GPRegisters {
                rax: 0,
                rbx: 0,
                rcx: 0,
                rdx: 0,
                rsi: 0,
                rdi: 0,
                rbp: 0,
                rsp: 0,
                r8: 0,
                r9: 0,
                r10: 0,
                r11: 0,
                r12: 0,
                r13: 0,
                r14: 0,
                r15: 0,
            },
            rip: Default::default(),
            rflags: RFlags::FLAGS_IF,
            cs: Default::default(),
            ss: Default::default(),
        }
    }
}

#[unsafe(naked)]
// rdi, rsi, rdx, rcx, r8, r9
unsafe extern "C" fn jump_to_context(
    buf: *const GPRegisters,
    ss: u64,
    rsp: u64,
    rflags: u64,
    cs: u64,
    rip: u64,
) -> ! {
    // TODO: we need to switch ds here
    // maybe also fs
    // and maybe everything else
    // oh well it's up to John Userspace to do this
    naked_asm!(
        "pushq %rsi",
        "pushq %rdx",
        "pushq %rcx",
        "pushq %r8",
        "pushq %r9",
        // oh god this routine gives me flashbacks
        "movq (0 * 8)(%rdi), %rax",
        "movq (1 * 8)(%rdi), %rbx",
        "movq (2 * 8)(%rdi), %rcx",
        "movq (3 * 8)(%rdi), %rdx",
        "movq (4 * 8)(%rdi), %rsi",
        // "movq (5 * 8)(%rdi), %rdi", MOVED
        "movq (6 * 8)(%rdi), %rbp",
        // "movq (7 * 8)(%rdi), %rsp", BAD BAD BAD don't load sp
        "movq (8 * 8)(%rdi), %r8",
        "movq (9 * 8)(%rdi), %r9",
        "movq (10 * 8)(%rdi), %r10",
        "movq (11 * 8)(%rdi), %r11",
        "movq (12 * 8)(%rdi), %r12",
        "movq (13 * 8)(%rdi), %r13",
        "movq (14 * 8)(%rdi), %r14",
        "movq (15 * 8)(%rdi), %r15",
        // don't clobber registers!
        "movq (5 * 8)(%rdi), %rdi",
        // why use iretq? because of the woke left. just kidding. it makes handling DPL easier once you get userspace :D
        "iretq",
        options(att_syntax)
    )
}

impl Context {
    pub fn jump_to(&self) -> ! {
        unsafe {
            jump_to_context(
                &raw const self.gp,
                self.ss,
                self.gp.rsp,
                self.rflags.bits(),
                self.cs,
                self.rip,
            )
        }
    }

    pub fn setup_kthread_context(&mut self) {
        self.cs = SegmentSelector::new(GlobalDescriptorTable::CS, Ring::Ring0)
            .bits()
            .into();

        self.ss = SegmentSelector::new(GlobalDescriptorTable::DS, Ring::Ring0)
            .bits()
            .into();
    }

    pub fn setup_for_call<T>(
        &mut self,
        stack: &[u8],
        function: unsafe extern "C" fn(*mut T) -> !,
        data: *mut T,
    ) {
        self.setup_kthread_context();

        self.rip = function as usize as u64;
        self.gp.rdi = data as u64;
        self.gp.rsp = slice_stack_pointer(stack);
    }
}

#[unsafe(naked)]
unsafe extern "C" fn save_context_impl(buf: *mut u64) -> u64 {
    // Functions preserve the registers rbx, rsp, rbp, r12, r13, r14, and r15
    naked_asm!(
        "movq %rbx, (0 * 8)(%rdi)",
        "movq %rsp, (1 * 8)(%rdi)",
        "movq %rbp, (2 * 8)(%rdi)",
        "movq %r12, (3 * 8)(%rdi)",
        "movq %r13, (4 * 8)(%rdi)",
        "movq %r14, (5 * 8)(%rdi)",
        "movq %r15, (6 * 8)(%rdi)",
        "leaq .L0(%rip), %rax",
        "movq %rax, (7 * 8)(%rdi)",
        "movq $0, %rax",
        "ret",
        ".L0:",
        "movq $1, %rax",
        "ret",
        options(att_syntax)
    )
}

pub unsafe fn save_context(ctx: &mut Context) -> bool {
    let mut buf = [0u64; 8];

    if unsafe { save_context_impl(buf.as_mut_ptr()) } == 0 {
        ctx.gp.rbx = buf[0];
        ctx.gp.rsp = buf[1];
        ctx.gp.rbp = buf[2];
        ctx.gp.r12 = buf[3];
        ctx.gp.r13 = buf[4];
        ctx.gp.r14 = buf[5];
        ctx.gp.r15 = buf[6];
        ctx.rip = buf[7];
        false
    } else {
        true
    }
}
