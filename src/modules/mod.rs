use core::ptr::slice_from_raw_parts;

use limine::{file::File, request::ModuleRequest};
use proc_macros::CmdlineParsable;

use crate::cmdline::{CmdlineLexer, CmdlineParsable};

pub mod symbols;

#[derive(CmdlineParsable)]
enum ModuleCmdline {
    InternalNull,
    Symbols,
}

// Limine rejects duplicate requests with the same ID, so all module consumers
// in the kernel must share one ModuleRequest instance.
#[used]
#[unsafe(link_section = ".limine_requests")]
pub(crate) static MODULE_REQUEST: ModuleRequest = ModuleRequest::new();

pub fn loaded_modules() -> &'static [&'static File] {
    MODULE_REQUEST
        .get_response()
        .map(|res| res.modules())
        .unwrap_or(&[])
}

pub fn find_by_cmdline(cmdline: &[u8]) -> Option<&'static File> {
    loaded_modules()
        .iter()
        .copied()
        .find(|module| module.string().to_bytes() == cmdline)
}

pub fn loaded_module_cmdlines() -> impl Iterator<Item = &'static [u8]> {
    loaded_modules()
        .iter()
        .map(|module| module.string().to_bytes())
}

pub fn module_data(module: &'static File) -> &'static [u8] {
    unsafe { &*slice_from_raw_parts(module.addr(), module.size() as usize) }
}

pub fn module_range(module: &'static File) -> (*mut u8, usize) {
    (module.addr(), module.size() as usize)
}

pub fn load_modules_early() {
    for module in loaded_modules() {
        let cmdline_str = match module.string().to_str() {
            Ok(x) => x,
            Err(_) => continue,
        };

        let mut cmdline = ModuleCmdline::InternalNull;

        match CmdlineLexer::parse(cmdline_str, &mut cmdline) {
            Ok(_) => {}
            Err(_) => continue,
        };

        match cmdline {
            ModuleCmdline::InternalNull => {
                continue;
            }
            ModuleCmdline::Symbols => {
                let Some(syms) = symbols::parse(module_data(module)) else {
                    continue;
                };

                symbols::try_init(syms);
            }
        }
    }
}
