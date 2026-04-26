use virtio_drivers::{device::gpu::VirtIOGpu, transport::Transport};

use crate::devices::{
    Device,
    display::{DisplayDevice, DisplayError, DisplayInfo, PixelFormat},
    virtio::VirtioHal,
};

pub struct VirtIOGpuDriver<T: Transport> {
    gpu: VirtIOGpu<VirtioHal, T>,
    info: Option<DisplayInfo>,
}

impl<T: Transport> VirtIOGpuDriver<T> {
    pub fn new(transport: T) -> Result<Self, DisplayError> {
        let gpu = VirtIOGpu::<VirtioHal, T>::new(transport).map_err(|_| DisplayError::IoError)?;
        Ok(Self { gpu, info: None })
    }
}

impl<T: Transport> DisplayDevice for VirtIOGpuDriver<T> {
    fn name(&self) -> &str {
        "virtio_gpu"
    }

    fn setup(&mut self) -> Result<DisplayInfo, DisplayError> {
        let (width, height) = self.gpu.resolution().map_err(|_| DisplayError::IoError)?;
        let _ = self
            .gpu
            .setup_framebuffer()
            .map_err(|_| DisplayError::IoError)?;

        let info = DisplayInfo {
            width,
            height,
            stride: width * 4,
            format: PixelFormat::Bgra8888,
        };
        self.info = Some(info);
        Ok(info)
    }

    fn get_info(&self) -> Result<DisplayInfo, DisplayError> {
        self.info.ok_or(DisplayError::NotReady)
    }

    fn get_framebuffer(&mut self) -> Result<&mut [u8], DisplayError> {
        self.gpu
            .setup_framebuffer()
            .map_err(|_| DisplayError::IoError)
    }

    fn flush(&mut self) -> Result<(), DisplayError> {
        self.gpu.flush().map_err(|_| DisplayError::IoError)
    }
}

impl<T: Transport> Device for VirtIOGpuDriver<T> {
    fn ioctl(&self, _request: u64, _arg1: u64, _arg2: u64) -> u64 {
        0
    }
}
