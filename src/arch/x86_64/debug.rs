use x86::io::outw;

//again, copied from https://os.phil-opp.com/testing/
pub fn shutdown(err_code : u16) {
    unsafe{
        outw(0xf4, err_code);
        crate::arch::x86_64::asm::sleep_core();
    }    
}