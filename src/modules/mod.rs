use core::ptr::slice_from_raw_parts;

use crate::cmdline::{CmdlineLexer, CmdlineParsable};
use limine::request::ModuleRequest;
use proc_macros::CmdlineParsable;

pub mod symbols;

#[derive(CmdlineParsable)]
enum ModuleCmdline {
    InternalNull,
    Symbols,
}

#[used]
#[unsafe(link_section = ".limine_requests")]
static MODULE_REQUEST: ModuleRequest = ModuleRequest::new();

pub fn load_modules_early() {
    if let Some(res) = MODULE_REQUEST.get_response() {
        for module in res.modules() {
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
                    let Some(syms) = symbols::parse(unsafe {
                        &*slice_from_raw_parts(module.addr(), module.size() as usize)
                    }) else {
                        continue;
                    };

                    symbols::try_init(syms);
                }
            }
        }
    }
}
