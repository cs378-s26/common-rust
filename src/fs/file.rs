use alloc::{string::String, sync::Arc};

#[derive(Debug)]
pub enum FileError {
    NotSupported,
    WouldBlock,
    BadFileDescriptor,
    Other(String),
}

// Anything that can be stored in a file descriptor table implements this.
// Send + Sync required because Process is shared across threads via Arc.
pub trait File: Send + Sync {
    fn read(&self, buf: &mut [u8]) -> Result<usize, FileError>;
    fn write(&self, buf: &[u8]) -> Result<usize, FileError>;
    fn close(&self) -> Result<(), FileError>;

    // Erase to Arc<dyn Any> so syscalls like recvfrom can downcast to a
    // concrete type (e.g. Arc<UdpSocket>) when they need socket-specific methods.
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn core::any::Any + Send + Sync>;
}
