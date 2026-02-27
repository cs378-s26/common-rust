use core::fmt::{
    self, Binary, Debug, Display, Formatter, LowerExp, LowerHex, Octal, Pointer, Result, UpperExp,
    UpperHex, Write,
};
use core::ptr;
use flanterm::{
    flanterm_context, flanterm_fb_init, flanterm_flush, flanterm_set_autoflush, flanterm_write,
};

use bitflags::bitflags;
use limine::framebuffer::Framebuffer;
use limine::request::FramebufferRequest;
use spin::Once;

use crate::arch::{self, SerialCharSink, UnwindContext};
use crate::sync::IntMutex;

#[derive(Clone, Copy)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub const fn rgb(&self) -> u32 {
        ((self.0 as u32) << 16) | ((self.1 as u32) << 8) | (self.2 as u32)
    }

    pub const fn from_rgb(data: u32) -> Color {
        Color((data >> 16) as u8, (data >> 8) as u8, data as u8)
    }

    pub const BACKGROUND: Color = Self::from_rgb(0x131a1c);
    pub const FOREGROUND: Color = Self::from_rgb(0xc5c8c9);
    pub const CURSOR: Color = Self::from_rgb(0x808080);

    pub const BLACK: Color = Self::from_rgb(0x131a1c);
    pub const RED: Color = Self::from_rgb(0xe74c4c);
    pub const GREEN: Color = Self::from_rgb(0x6bb05d);
    pub const YELLOW: Color = Self::from_rgb(0xe59e67);
    pub const BLUE: Color = Self::from_rgb(0x5b98a9);
    pub const PURPLE: Color = Self::from_rgb(0xb185db);
    pub const CYAN: Color = Self::from_rgb(0x51a39f);
    pub const WHITE: Color = Self::from_rgb(0xc4c4c4);

    pub const BRIGHT_BLACK: Color = Self::from_rgb(0x343636);
    pub const BRIGHT_RED: Color = Self::from_rgb(0xc26f6f);
    pub const BRIGHT_GREEN: Color = Self::from_rgb(0x8dc776);
    pub const BRIGHT_YELLOW: Color = Self::from_rgb(0xe7ac7e);
    pub const BRIGHT_BLUE: Color = Self::from_rgb(0x7ab3c3);
    pub const BRIGHT_PURPLE: Color = Self::from_rgb(0xbb84e5);
    pub const BRIGHT_CYAN: Color = Self::from_rgb(0x6db0ad);
    pub const BRIGHT_WHITE: Color = Self::from_rgb(0xcccccc);

    pub fn format<'a, T>(&self, data: &'a T) -> ANSIFormatter<'a, T> {
        let mut fmt = ANSIFormatter::new(data);
        fmt.color(*self);
        fmt
    }
}

bitflags! {
    struct ANSIFormatFlags: u8 {
        const BOLD = 1 << 0;
        const ITALIC = 1 << 1;
    }
}

pub struct ANSIFormatter<'a, T> {
    data: &'a T,
    flags: ANSIFormatFlags,
    color: Option<Color>,
}

impl<'a, T> ANSIFormatter<'a, T> {
    pub fn new(data: &'a T) -> ANSIFormatter<'a, T> {
        ANSIFormatter {
            data,
            flags: ANSIFormatFlags::empty(),
            color: None,
        }
    }

    pub fn color(&mut self, color: Color) -> &mut Self {
        self.color = Some(color);
        self
    }

    pub fn bold(&mut self) -> &mut Self {
        self.flags.insert(ANSIFormatFlags::BOLD);
        self
    }

    pub fn italic(&mut self) -> &mut Self {
        self.flags.insert(ANSIFormatFlags::ITALIC);
        self
    }
}

macro impl_for($trait:ident) {
    impl<'a, T: $trait> $trait for ANSIFormatter<'a, T> {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            if self.flags.contains(ANSIFormatFlags::BOLD) {
                f.write_str("\x1b[1m")?;
            }

            if self.flags.contains(ANSIFormatFlags::ITALIC) {
                f.write_str("\x1b[3m")?;
            }

            if let Some(color) = &self.color {
                write!(f, "\x1b[38;2;{};{};{}m", color.0, color.1, color.2)?;
            }

            self.data.fmt(f)?;

            f.write_str("\x1b[0m")
        }
    }
}

impl_for!(Display);
impl_for!(Debug);
impl_for!(Octal);
impl_for!(LowerHex);
impl_for!(UpperHex);
impl_for!(Pointer);
impl_for!(Binary);
impl_for!(LowerExp);
impl_for!(UpperExp);

pub trait CharSink: Send + Sync {
    unsafe fn putc(&self, ch: u8);

    unsafe fn flush(&self);
}

pub struct FlanTermSink(*mut flanterm_context);

impl FlanTermSink {
    pub fn from_framebuffer(fb: &Framebuffer) -> FlanTermSink {
        let context: *mut flanterm_context;
        let mut ansi_colors = [
            Color::BLACK.rgb(),
            Color::RED.rgb(),
            Color::GREEN.rgb(),
            Color::YELLOW.rgb(),
            Color::BLUE.rgb(),
            Color::PURPLE.rgb(),
            Color::CYAN.rgb(),
            Color::WHITE.rgb(),
        ];

        let mut ansi_colors_bright = [
            Color::BRIGHT_BLACK.rgb(),
            Color::BRIGHT_RED.rgb(),
            Color::BRIGHT_GREEN.rgb(),
            Color::BRIGHT_YELLOW.rgb(),
            Color::BRIGHT_BLUE.rgb(),
            Color::BRIGHT_PURPLE.rgb(),
            Color::BRIGHT_CYAN.rgb(),
            Color::BRIGHT_WHITE.rgb(),
        ];

        let mut default_bg = Color::BACKGROUND.rgb();
        let mut default_fg = Color::FOREGROUND.rgb();

        unsafe {
            context = flanterm_fb_init(
                None,
                None,
                fb.addr() as *mut u32,
                usize::try_from(fb.width()).unwrap(),
                usize::try_from(fb.height()).unwrap(),
                usize::try_from(fb.pitch()).unwrap(),
                fb.red_mask_size(),
                fb.red_mask_shift(),
                fb.green_mask_size(),
                fb.green_mask_shift(),
                fb.blue_mask_size(),
                fb.blue_mask_shift(),
                ptr::null_mut(),
                ansi_colors.as_mut_ptr(),
                ansi_colors_bright.as_mut_ptr(),
                &raw mut default_bg,
                &raw mut default_fg,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0usize,
                0usize,
                1usize,
                0usize,
                0usize,
                0usize,
                0i32,
            );

            flanterm_set_autoflush(context, false);
        }

        FlanTermSink(context)
    }
}

impl CharSink for FlanTermSink {
    unsafe fn putc(&self, ch: u8) {
        unsafe {
            let cch = ch as core::ffi::c_char;
            flanterm_write(self.0, ptr::from_ref(&cch), 1);

            if ch == b'\n' {
                flanterm_flush(self.0);
            }
        }
    }

    unsafe fn flush(&self) {
        unsafe { flanterm_flush(self.0) };
    }
}

unsafe impl Send for FlanTermSink {}
unsafe impl Sync for FlanTermSink {}

// TODO: this is not very rusty
static LOCK_PW: IntMutex<()> = IntMutex::new(());
pub static LOCK_KPRINT: IntMutex<()> = IntMutex::new(());
static FLAN_TERM_BACKEND: Once<FlanTermSink> = Once::new();
static SERIAL_BACKEND: Once<SerialCharSink> = Once::new();

pub struct PrintWriter;

impl Write for PrintWriter {
    fn write_str(&mut self, s: &str) -> Result {
        let _guard = LOCK_PW.lock();

        if let Some(ft) = FLAN_TERM_BACKEND.get() {
            for ele in s.bytes() {
                unsafe { ft.putc(ele) };
            }
        }

        if let Some(ft) = SERIAL_BACKEND.get() {
            for ele in s.bytes() {
                unsafe { ft.putc(ele) };
            }
        }

        Ok(())
    }
}

#[used]
#[unsafe(link_section = ".limine_requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

pub fn init_tty() {
    if let Some(res) = FRAMEBUFFER_REQUEST.get_response()
        && let Some(ref fb) = res.framebuffers().next()
    {
        FLAN_TERM_BACKEND.call_once(|| FlanTermSink::from_framebuffer(fb));
    }

    kprintln!("init_tty(): tty initialized");

    if let Some(res) = FRAMEBUFFER_REQUEST.get_response()
        && let Some(ref fb) = res.framebuffers().next()
    {
        kprintln!("init_tty(): framebuffer: {}x{}", fb.width(), fb.height());
    }

    arch::init_tty(&SERIAL_BACKEND);
}

pub macro kprint {
    ($($arg:tt)*) => {{
        use $crate::print::PrintWriter;
        let _guard = $crate::print::LOCK_KPRINT.lock();
        let _ = PrintWriter.write_fmt(::core::format_args!($($arg)*));
    }}
}

pub macro kprintln {
    () => {{
        $mod::kprint!("\n");
    }},
    ($fmt:expr) => {{
        $crate::print::kprint!(concat!($fmt, "\n"));
    }},
    ($fmt:expr, $($arg:tt)*) => {{
        $crate::print::kprint!(concat!($fmt, "\n"), $($arg)*);
    }}
}

pub struct StackTrace(UnwindContext);

impl StackTrace {
    pub fn new(ctx: UnwindContext) -> StackTrace {
        StackTrace(ctx)
    }

    #[inline(always)]
    pub fn current() -> StackTrace {
        Self::new(unsafe { UnwindContext::get() })
    }
}

impl Display for StackTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result {
        let StackTrace(mut context) = *self;

        let mut i = 0;
        while unsafe { context.valid() } {
            let addr = unsafe { context.return_address() };
            writeln!(f, "#{}: {:#016x}", i, addr)?;
            i += 1;
            context = unsafe { context.next() };
        }

        Ok(())
    }
}
