// SycallContext Trait
// The purpose of this trait is to unify system calls between
// x86 and ARM such that each architecture can implement this
// trait for their respective interrupt context structs.
// Thus, the syscall handler will only have to deal with a
// struct implementing SycallContext, agnostic of architecture

pub trait SyscallContext {
    // syscall number
    // x8 on ARM - rax on x86
    fn syscall_number(&self) -> usize;

    // max of 6 arguments to a unix sycall
    fn arg0(&self) -> usize;
    fn arg1(&self) -> usize;
    fn arg2(&self) -> usize;
    fn arg3(&self) -> usize;
    fn arg4(&self) -> usize;
    fn arg5(&self) -> usize;

    fn get_arg(&self, n: usize) -> Option<usize> {
        match n {
            0 => Some(self.arg0()),
            1 => Some(self.arg1()),
            2 => Some(self.arg2()),
            3 => Some(self.arg3()),
            4 => Some(self.arg4()),
            5 => Some(self.arg5()),
            _ => None,
        }
    }

    fn set_return_value(&mut self, ret: usize);

    // fn is_user_address(&self, ptr: usize) -> bool;

    // fn get_arg_ptr_safe(&self, n: usize) -> Option<usize> {
    //     let potential_pointer = self.get_arg(n)?;
    //     if self.is_user_address(potential_pointer) {
    //         Some(potential_pointer)
    //     } else {
    //         None
    //     }
    // }
}

pub fn syscall_handler(syscall_context: &mut impl SyscallContext) {
    // https://filippo.io/linux-syscall-table/
    // NOTE: x86 and ARM syscall numbers differ in the unix spec
    // Ex: 0 is read() on x86 while 63 is read() on ARM
    // We can write two syscall handlers depending on the target. This will still let us share the syscall functions.
    // We could also implement some sort of higher translation between number and some sycall enum to avoid this.

    match syscall_context.syscall_number() {
        0 => panic!("TRIED TO SYSCALL READ"),
        _ => panic!("SYSCALLS UNIMPLEMENTED"),
    }
}
