pub mod virtio_gpu;

use crate::devices::Device;

#[derive(Debug, Clone, Copy)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
}

#[derive(Debug, Clone, Copy)]
pub enum PixelFormat {
    Bgra8888,
}

impl PixelFormat {
    pub fn bytes_per_pixel(&self) -> u32 {
        match self {
            PixelFormat::Bgra8888 => 4,
        }
    }
}

#[derive(Debug)]
pub enum DisplayError {
    NotReady,
    IoError,
    InvalidParam,
    Unsupported,
}

pub trait DisplayDevice: Device {
    fn name(&self) -> &str;
    fn setup(&mut self) -> Result<DisplayInfo, DisplayError>;
    fn get_info(&self) -> Result<DisplayInfo, DisplayError>;
    fn get_framebuffer(&mut self) -> Result<&mut [u8], DisplayError>;
    fn flush(&mut self) -> Result<(), DisplayError>;
}

pub fn test_framebuffer() {
    use crate::devices::discovery::DISPLAY_DEVICES;
    use crate::print::kprintln;
    use crate::sync::MutexLike;

    kprintln!("[fb_test] Starting framebuffer test...");

    let mut displays = DISPLAY_DEVICES.lock();
    if displays.is_empty() {
        kprintln!("[fb_test] No display devices found!");
        return;
    }

    let display = &mut *displays[0];
    kprintln!("[fb_test] Found display: {}", display.name());

    let info = match display.setup() {
        Ok(i) => i,
        Err(e) => {
            kprintln!("[fb_test] Failed to setup display: {:?}", e);
            return;
        }
    };

    kprintln!(
        "[fb_test] Resolution: {}x{}, stride: {}, format: {:?}",
        info.width, info.height, info.stride, info.format
    );

    let fb = match display.get_framebuffer() {
        Ok(fb) => fb,
        Err(e) => {
            kprintln!("[fb_test] Failed to get framebuffer: {:?}", e);
            return;
        }
    };

    kprintln!("[fb_test] Drawing test pattern...");
    let width = info.width as usize;
    let height = info.height as usize;
    let bpp = info.format.bytes_per_pixel() as usize;

    for y in 0..height {
        for x in 0..width {
            let offset = y * info.stride as usize + x * bpp;
            if offset + 3 < fb.len() {
                let r = ((x * 255) / width) as u8;
                let g = ((y * 255) / height) as u8;
                let b = (((x + y) * 255) / (width + height)) as u8;
                // BGRA format
                fb[offset] = b;
                fb[offset + 1] = g;
                fb[offset + 2] = r;
                fb[offset + 3] = 255;
            }
        }
    }

    if let Err(e) = display.flush() {
        kprintln!("[fb_test] Failed to flush: {:?}", e);
        return;
    }

    kprintln!("[fb_test] Framebuffer test complete! You should see a color gradient.");
}
