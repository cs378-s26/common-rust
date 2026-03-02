mod lexer;
mod parse;

use core::{cell::SyncUnsafeCell, str::Utf8Error};

pub use lexer::*;
pub use parse::*;

use limine::request::ExecutableCmdlineRequest;
use proc_macros::CmdlineParsable;
use spin::Once;

#[derive(Clone, Copy)]
pub struct KernelCmdline {}

impl CmdlineParsable for KernelCmdline {
    fn parse<'a>(&mut self, lexer: &mut CmdlineLexer<'a>) -> Result<(), CmdlineParseError<'a>> {
        lexer.parse_block(CmdlineTokenData::Eof, CmdlineTokenData::Comma, |lexer| {
            let tok = lexer.next_tok()?;
            tok.unwrap_ident()?;
            Err(tok.make_error(CmdlineErrorCode::UnknownField(&[])))
        })
    }
}

// requests

#[used]
#[unsafe(link_section = ".limine_requests")]
static CMDLINE_REQUEST: ExecutableCmdlineRequest = ExecutableCmdlineRequest::new();

static DEFAULT_OPTIONS: KernelCmdline = KernelCmdline {};

pub enum CmdlineError {
    NoResponse,
    Utf8Error(Utf8Error),
    ParseError(CmdlineParseError<'static>),
}

static CMDLINE_TEXT: Once<&'static str> = Once::new();
// need to use SyncUnsafeCell here because we need a mutable ref for parse to avoid stack
// allocations
static CMDLINE_STATE: SyncUnsafeCell<KernelCmdline> = SyncUnsafeCell::new(DEFAULT_OPTIONS);
static CMDLINE_ERROR: Once<CmdlineError> = Once::new();

pub fn get_cmdline() -> &'static KernelCmdline {
    unsafe { &*CMDLINE_STATE.get() }
}

pub fn get_cmdline_text() -> Option<&'static str> {
    CMDLINE_TEXT.get().map(|v| &**v)
}

pub fn get_cmdline_error() -> Option<&'static CmdlineError> {
    CMDLINE_ERROR.get()
}

pub fn parse_kernel_cmdline() {
    let state = unsafe { &mut *CMDLINE_STATE.get() };

    if let Some(res) = CMDLINE_REQUEST.get_response() {
        let res = match res.cmdline().to_str() {
            Ok(x) => x,
            Err(err) => {
                CMDLINE_ERROR.call_once(|| CmdlineError::Utf8Error(err));
                return;
            }
        };

        match CmdlineLexer::parse(res, state) {
            Ok(_) => {}
            Err(err) => {
                // reset to default
                *state = DEFAULT_OPTIONS;
                CMDLINE_ERROR.call_once(|| CmdlineError::ParseError(err));
            }
        }
    } else {
        CMDLINE_ERROR.call_once(|| CmdlineError::NoResponse);
    }
}
