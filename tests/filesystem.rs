#![no_std]
#![no_main]
#![feature(decl_macro)]
#![feature(const_trait_impl)]
#![feature(const_default)]
#![feature(slice_ptr_get)]
#![feature(box_as_ptr)]
#![feature(const_range)]
#![feature(never_type)]
#![feature(sync_unsafe_cell)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use kernel_common::MP_REQUEST;
use kernel_common::arch::{Arch, ArchTrait, KernelEntryTrait};
use kernel_common::coroutine::{init_coroutine_executor, init_coroutine_queue};
use kernel_common::devices::block::{BlockDevice, BlockError, PhysicalAddressSize};
use kernel_common::filessytem::ext2::Ext2;
use kernel_common::mp::{MP_STAGE, MPStage};
use kernel_common::print::kprintln;
use kernel_common::thread::{poll_tasks, set_up_idle, spawn_thread, yield_thread};
use spin::{Barrier, Once};

#[cfg(test)]
static INIT_THREADING_BARRIER: Once<Barrier> = Once::new();
#[cfg(test)]
static MP_PREEMPT_ENTER_BARRIER: Once<Barrier> = Once::new();
#[cfg(test)]
static MAKE_TEST_THREAD: Once<()> = Once::new();
#[cfg(test)]
pub struct TestKernelEntry;
#[cfg(test)]
impl KernelEntryTrait for TestKernelEntry {
    fn kernel_main() -> ! {
        let mp_res = MP_REQUEST
            .get_response()
            .expect("Expected to find MpResponse, found None.");
        let core_count = mp_res.cpus().len();

        INIT_THREADING_BARRIER
            .call_once(|| {
                init_coroutine_queue();
                Barrier::new(core_count)
            })
            .wait();

        set_up_idle();

        init_coroutine_executor();
        kprintln!("Coroutine executor initialized.");

        MP_PREEMPT_ENTER_BARRIER
            .call_once(|| Barrier::new(core_count))
            .wait();

        MP_STAGE.store(MPStage::MPPreempt, Ordering::SeqCst);

        MAKE_TEST_THREAD.call_once(|| {
            spawn_thread(move || {
                kprintln!("Starting Testing Code...");
                crate::test_main();
            })
        });

        Arch::set_irq_enabled(true);
        poll_tasks()
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system_main() -> ! {
    kernel_common::system_init::<Arch, TestKernelEntry>();
}

#[panic_handler]
fn rust_panic(info: &core::panic::PanicInfo) -> ! {
    kernel_common::test_utils::rust_panic_test_impl(info);
}

const DEVICE_BLOCK_SIZE: usize = 512;
const FS_BLOCK_SIZE: usize = 1024;
const FS_BLOCK_COUNT: usize = 64;
const INODE_SIZE: usize = 128;
const INODES_PER_GROUP: usize = 32;

const ROOT_INODE: usize = 2;
const ETC_INODE: usize = 3;
const HELLO_INODE: usize = 4;
const NESTED_INODE: usize = 5;

const ROOT_DIR_BLOCK: usize = 7;
const ETC_DIR_BLOCK: usize = 8;
const HELLO_BLOCK: usize = 9;
const NESTED_BLOCK: usize = 10;

const HELLO_CONTENT: &[u8] = b"Hello from ext2!\n";
const NESTED_CONTENT: &[u8] = b"nested file\n";
const CONCURRENT_READERS: usize = 8;

static READER_LATCH: AtomicUsize = AtomicUsize::new(0);

// Minimal in-memory device used to unit-test the filesystem layer without real hardware.
struct MockBlockDevice {
    data: Vec<u8>,
    block_size: usize,
}

impl MockBlockDevice {
    fn new(block_size: usize, data: Vec<u8>) -> Self {
        assert_eq!(data.len() % block_size, 0);
        Self { data, block_size }
    }
}

impl BlockDevice for MockBlockDevice {
    fn init() -> Result<(), BlockError> {
        Ok(())
    }

    fn read_blocks(
        &self,
        block_idxs: &[usize],
        buffers: &mut [&mut [u8]],
    ) -> Result<(), BlockError> {
        if block_idxs.len() != buffers.len() {
            return Err(BlockError::InvalidBufferSize);
        }

        for (block_idx, out) in block_idxs.iter().zip(buffers.iter_mut()) {
            if out.len() != self.block_size {
                return Err(BlockError::InvalidBufferSize);
            }

            let start = block_idx
                .checked_mul(self.block_size)
                .ok_or(BlockError::InvalidBlockIndex)?;
            let end = start
                .checked_add(self.block_size)
                .ok_or(BlockError::InvalidBlockIndex)?;
            if end > self.data.len() {
                return Err(BlockError::InvalidBlockIndex);
            }

            out.copy_from_slice(&self.data[start..end]);
        }

        Ok(())
    }

    fn write_blocks(&self, _block_idxs: &[usize], _buffer: &[&[u8]]) -> Result<(), BlockError> {
        Err(BlockError::DeviceError)
    }

    fn flush(&self) -> Result<(), BlockError> {
        Ok(())
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn block_count(&self) -> usize {
        self.data.len() / self.block_size
    }

    fn dma_physical_address_size(&self) -> PhysicalAddressSize {
        PhysicalAddressSize::Size64
    }
}

fn write_le_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_le_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn set_bitmap_bit(bitmap: &mut [u8], bit_idx: usize) {
    let byte_idx = bit_idx / 8;
    let bit = bit_idx % 8;
    bitmap[byte_idx] |= 1 << bit;
}

// Encode one inode entry in the inode table used by this synthetic image.
fn write_inode(
    image: &mut [u8],
    inode_number: usize,
    mode: u16,
    size: u32,
    links: u16,
    blocks: &[u32],
) {
    let inode_table_offset = 5 * FS_BLOCK_SIZE;
    let inode_offset = inode_table_offset + (inode_number - 1) * INODE_SIZE;
    let inode = &mut image[inode_offset..inode_offset + INODE_SIZE];

    write_le_u16(inode, 0, mode);
    write_le_u32(inode, 4, size);
    write_le_u16(inode, 26, links);
    write_le_u32(inode, 28, (blocks.len() as u32) * 2);

    for (idx, block) in blocks.iter().enumerate() {
        write_le_u32(inode, 40 + idx * 4, *block);
    }
}

// Encode one ext2 directory entry. Caller chooses rec_len so final record can consume remainder.
fn write_dir_entry(
    block: &mut [u8],
    offset: usize,
    inode: u32,
    name: &[u8],
    file_type: u8,
    rec_len: usize,
) -> usize {
    write_le_u32(block, offset, inode);
    write_le_u16(block, offset + 4, rec_len as u16);
    block[offset + 6] = name.len() as u8;
    block[offset + 7] = file_type;
    block[offset + 8..offset + 8 + name.len()].copy_from_slice(name);
    offset + rec_len
}

// Build a tiny valid ext2 image:
// - 1KiB filesystem blocks
// - root dir with /hello.txt and /etc
// - /etc/nested.txt
// The image is intentionally minimal but structurally correct for parser tests.
fn create_test_ext2_image() -> Vec<u8> {
    let mut image = vec![0_u8; FS_BLOCK_COUNT * FS_BLOCK_SIZE];

    // Superblock starts at byte 1024 for ext2.
    let superblock = &mut image[FS_BLOCK_SIZE..2 * FS_BLOCK_SIZE];
    write_le_u32(superblock, 0, INODES_PER_GROUP as u32);
    write_le_u32(superblock, 4, FS_BLOCK_COUNT as u32);
    write_le_u32(superblock, 12, (FS_BLOCK_COUNT - 11) as u32);
    write_le_u32(superblock, 16, (INODES_PER_GROUP - 5) as u32);
    write_le_u32(superblock, 20, 1);
    write_le_u32(superblock, 24, 0);
    write_le_u32(superblock, 32, FS_BLOCK_COUNT as u32);
    write_le_u32(superblock, 40, INODES_PER_GROUP as u32);
    write_le_u16(superblock, 56, 0xEF53);
    write_le_u16(superblock, 58, 1);
    write_le_u16(superblock, 60, 1);
    write_le_u32(superblock, 76, 1);
    write_le_u32(superblock, 84, 11);
    write_le_u16(superblock, 88, INODE_SIZE as u16);

    // Block group descriptor table (single group in this image).
    let bgdt_offset = 2 * FS_BLOCK_SIZE;
    write_le_u32(&mut image, bgdt_offset, 3);
    write_le_u32(&mut image, bgdt_offset + 4, 4);
    write_le_u32(&mut image, bgdt_offset + 8, 5);
    write_le_u16(&mut image, bgdt_offset + 12, (FS_BLOCK_COUNT - 11) as u16);
    write_le_u16(&mut image, bgdt_offset + 14, (INODES_PER_GROUP - 5) as u16);
    write_le_u16(&mut image, bgdt_offset + 16, 2);

    // Mark metadata + data blocks/inodes as used.
    let block_bitmap = &mut image[3 * FS_BLOCK_SIZE..4 * FS_BLOCK_SIZE];
    for used_block in 0..=10 {
        set_bitmap_bit(block_bitmap, used_block);
    }

    let inode_bitmap = &mut image[4 * FS_BLOCK_SIZE..5 * FS_BLOCK_SIZE];
    for used_inode in 0..=4 {
        set_bitmap_bit(inode_bitmap, used_inode);
    }

    // Root directory, /etc directory, and two regular files.
    write_inode(
        &mut image,
        ROOT_INODE,
        0x41ED,
        FS_BLOCK_SIZE as u32,
        3,
        &[ROOT_DIR_BLOCK as u32],
    );
    write_inode(
        &mut image,
        ETC_INODE,
        0x41ED,
        FS_BLOCK_SIZE as u32,
        2,
        &[ETC_DIR_BLOCK as u32],
    );
    write_inode(
        &mut image,
        HELLO_INODE,
        0x81A4,
        HELLO_CONTENT.len() as u32,
        1,
        &[HELLO_BLOCK as u32],
    );
    write_inode(
        &mut image,
        NESTED_INODE,
        0x81A4,
        NESTED_CONTENT.len() as u32,
        1,
        &[NESTED_BLOCK as u32],
    );

    // Root entries: ., .., etc, hello.txt
    let root_dir = &mut image[ROOT_DIR_BLOCK * FS_BLOCK_SIZE..(ROOT_DIR_BLOCK + 1) * FS_BLOCK_SIZE];
    let mut offset = 0;
    offset = write_dir_entry(root_dir, offset, ROOT_INODE as u32, b".", 2, 12);
    offset = write_dir_entry(root_dir, offset, ROOT_INODE as u32, b"..", 2, 12);
    offset = write_dir_entry(root_dir, offset, ETC_INODE as u32, b"etc", 2, 12);
    let final_len = FS_BLOCK_SIZE - offset;
    write_dir_entry(
        root_dir,
        offset,
        HELLO_INODE as u32,
        b"hello.txt",
        1,
        final_len,
    );

    // /etc entries: ., .., nested.txt
    let etc_dir = &mut image[ETC_DIR_BLOCK * FS_BLOCK_SIZE..(ETC_DIR_BLOCK + 1) * FS_BLOCK_SIZE];
    let mut offset = 0;
    offset = write_dir_entry(etc_dir, offset, ETC_INODE as u32, b".", 2, 12);
    offset = write_dir_entry(etc_dir, offset, ROOT_INODE as u32, b"..", 2, 12);
    let final_len = FS_BLOCK_SIZE - offset;
    write_dir_entry(
        etc_dir,
        offset,
        NESTED_INODE as u32,
        b"nested.txt",
        1,
        final_len,
    );

    // File data payloads.
    let hello_data = &mut image[HELLO_BLOCK * FS_BLOCK_SIZE..(HELLO_BLOCK + 1) * FS_BLOCK_SIZE];
    hello_data[..HELLO_CONTENT.len()].copy_from_slice(HELLO_CONTENT);

    let nested_data = &mut image[NESTED_BLOCK * FS_BLOCK_SIZE..(NESTED_BLOCK + 1) * FS_BLOCK_SIZE];
    nested_data[..NESTED_CONTENT.len()].copy_from_slice(NESTED_CONTENT);

    image
}

// TODO: update this test once we have driver support for x86 & aarch64
#[test_case]
fn test_read() {
    // Mount the synthetic image through the test block device.
    let image = create_test_ext2_image();
    let fs = Arc::new(
        Ext2::mount(MockBlockDevice::new(DEVICE_BLOCK_SIZE, image))
            .expect("failed to mount ext2 image"),
    );

    kprintln!("mounted ext2 block_size={}", fs.block_size());

    // Absolute path lookup + read validation.
    let hello_inode = fs
        .find(fs.root_inode(), "/hello.txt")
        .expect("failed absolute path lookup");
    let mut hello_buf = [0_u8; FS_BLOCK_SIZE];
    let hello_read = hello_inode
        .read(fs.as_ref(), 0, &mut hello_buf)
        .expect("failed to read hello.txt");
    assert_eq!(hello_read, HELLO_CONTENT.len());
    assert_eq!(&hello_buf[..hello_read], HELLO_CONTENT);
    kprintln!("absolute path lookup passed");

    // Relative lookup from /etc + read validation.
    let etc_inode = fs
        .find(fs.root_inode(), "/etc")
        .expect("failed to locate /etc");
    let hello_from_etc = fs
        .find(etc_inode, "/hello.txt")
        .expect("absolute path should ignore the provided start inode");
    assert_eq!(hello_from_etc.inode_number(), HELLO_INODE as u32);
    let nested_inode = etc_inode
        .find(fs.as_ref(), "nested.txt")
        .expect("failed relative path lookup");
    let mut nested_buf = [0_u8; FS_BLOCK_SIZE];
    let nested_read = nested_inode
        .read(fs.as_ref(), 0, &mut nested_buf)
        .expect("failed to read nested.txt");
    assert_eq!(nested_read, NESTED_CONTENT.len());
    assert_eq!(&nested_buf[..nested_read], NESTED_CONTENT);
    kprintln!("relative path lookup passed");
    kprintln!("file read passed");

    // Spawn multiple readers to ensure shared read-only access behaves under concurrency.
    READER_LATCH.store(0, Ordering::SeqCst);
    for _ in 0..CONCURRENT_READERS {
        let fs = Arc::clone(&fs);
        spawn_thread(move || {
            let inode = fs
                .find(fs.root_inode(), "/hello.txt")
                .expect("failed to locate /hello.txt in reader thread");
            let mut thread_buf = [0_u8; FS_BLOCK_SIZE];
            let read = inode
                .read(fs.as_ref(), 0, &mut thread_buf)
                .expect("failed to read /hello.txt in reader thread");
            assert_eq!(read, HELLO_CONTENT.len());
            assert_eq!(&thread_buf[..read], HELLO_CONTENT);
            READER_LATCH.fetch_add(1, Ordering::SeqCst);
        });
        yield_thread();
    }

    // Wait until all reader threads have completed.
    while READER_LATCH.load(Ordering::SeqCst) != CONCURRENT_READERS {
        yield_thread();
    }

    kprintln!("concurrent reads passed");
    kprintln!("filesystem test complete");
    Arch::shutdown(0);
}

// TODO: tests for indirection
// TODO: tests for files in multiple groups
