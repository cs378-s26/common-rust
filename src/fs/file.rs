use alloc::sync::Arc;

use crate::{
    fs::vfs::{FsError, VNode},
    sync::{IntMutex, MutexLike},
};

pub struct File {
    pub vnode: Arc<dyn VNode>,
    pub offset: IntMutex<usize>,
}

impl File {
    pub fn new(vnode: Arc<dyn VNode>) -> Self {
        Self {
            vnode,
            offset: IntMutex::new(0),
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize, FsError> {
        let mut offset = self.offset.lock();
        let bytes_read = self.vnode.read_unaligned(*offset, buf)?;
        *offset += bytes_read;
        Ok(bytes_read)
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize, FsError> {
        let mut offset = self.offset.lock();
        let bytes_written = self.vnode.write_unaligned(*offset, buf)?;
        *offset += bytes_written;
        Ok(bytes_written)
    }
}
