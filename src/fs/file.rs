use alloc::string::String;

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
}
