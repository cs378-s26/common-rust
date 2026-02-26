extern crate alloc;

use crate::print;
use limine::request::ModuleRequest;

#[used]
#[unsafe(link_section = ".limine_requests")]
static MODULE_REQUEST: ModuleRequest = ModuleRequest::new();

pub fn fs_init() {
    let res = MODULE_REQUEST
        .get_response()
        .expect("could not load modules (needed for fs)");
    let module = res.modules()[1];
    let addr = module.addr();
    let size = module.size();
    unsafe {
        let giggles = core::slice::from_raw_parts(addr as *const u16, (size / 2) as usize);
        print::kprintln!(
            "fs magic number: {}!",
            print::Color::YELLOW.format(&alloc::format!("0x{:X}", giggles[(1024 + 56) / 2]))
        );
    }
}
