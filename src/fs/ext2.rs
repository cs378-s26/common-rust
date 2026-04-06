extern crate alloc;

use super::ramdisk::Disk;
use crate::devices::block::BlockDevice;
use crate::sync::IntMutex;
use crate::sync::MutexLike;
use alloc::{collections::btree_map::BTreeMap, sync::Arc, sync::Weak};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

#[allow(non_camel_case_types)]
pub struct Ext2_old<D: Disk> {
    block_size: usize,
    disk: IntMutex<D>,
    superblock: Superblock,
    fnode_cache: IntMutex<BTreeMap<u32, Arc<FNode_old<D>>>>,
    block_map_lock: IntMutex<()>,
    inode_map_lock: IntMutex<()>,
    group_lock: IntMutex<()>,
}

#[allow(non_camel_case_types)]
pub struct FNode_old<D: Disk> {
    fs: Arc<Ext2_old<D>>,
    inode: IntMutex<INode>,
}

#[repr(C, packed)]
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
struct Superblock {
    inodes_count: u32,
    blocks_count: u32,
    r_blocks_count: u32,
    free_blocks_count: u32,
    free_inodes_count: u32,
    first_data_block: u32,
    log_block_size: u32,
    log_frag_size: u32,
    blocks_per_group: u32,
    frags_per_group: u32,
    inodes_per_group: u32,
    mtime: u32,
    wtime: u32,
    mnt_count: u16,
    max_mnt_count: u16,
    magic: u16,
    state: u16,
    errors: u16,
    minor_rev_level: u16,
    lastcheck: u32,
    checkinterval: u32,
    creator_os: u32,
    rev_level: u32,
    def_resuid: u16,
    def_resgid: u16,
    first_ino: u32,
    inode_size: u16,
    block_group_nr: u16,
    feature_compat: u32,
    feature_incompat: u32,
    feature_ro_compat: u32,
    uuid: [u8; 16],
    volume_name: [u8; 16],
    last_mounted: [u8; 64],
    algo_bitmap: u32,
    prealloc_blocks: u8,
    prealloc_dir_blocks: u8,
    _alignment: [u8; 2],
    journal_uuid: [u8; 16],
    journal_inum: u32,
    journal_dev: u32,
    last_orphan: u32,
    hash_seed: [u32; 4],
    def_has_version: u8,
    _padding: [u8; 3],
    default_mount_options: u32,
    first_meta_bg: u32,
}

#[repr(C, packed)]
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
struct BlockGroupDescriptor {
    block_bitmap: u32,
    inode_bitmap: u32,
    inode_table: u32,
    free_blocks_count: u16,
    free_inodes_count: u16,
    used_dirs_count: u16,
}

pub struct INode {
    number: usize,
    data: INodeData,
    #[allow(unused)]
    dirty: bool,
}

#[repr(C, packed)]
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
struct INodeData {
    mode: u16,
    uid: u16,
    size: u32,
    atime: u32,
    ctime: u32,
    mtime: u32,
    dtime: u32,
    gid: u16,
    links_count: u16,
    blocks: u32,
    flags: u32,
    osd1: u32,
    block: [u32; 15],
    generation: u32,
    file_acl: u32,
    dir_acl: u32,
    faddr: u32,
    osd2: [u8; 12],
}

pub struct Ext2<B: BlockDevice> {
    block_size: usize,
    block_device: IntMutex<B>,
    superblock: Superblock,
    fnode_cache: IntMutex<BTreeMap<u32, Arc<FNode<B>>>>,
    block_map_lock: IntMutex<()>,
    inode_map_lock: IntMutex<()>,
    group_lock: IntMutex<()>,
}

pub struct FNode<B: BlockDevice> {
    fs: Arc<Ext2<B>>,
    inode: IntMutex<INode>,
}

impl<B: BlockDevice> Ext2<B> {
    fn block_offset(&self, block_number: usize) -> usize {
        block_number * self.block_size
    }

    fn indirect_block_entry_offset(&self, block_number: usize, index: usize) -> usize {
        self.block_offset(block_number) + index * core::mem::size_of::<u32>()
    }

    fn read_bytes(&self, byte_offset: usize, buffer: &mut [u8]) {
        let mut block_device = self.block_device.lock();
        let bytes_read = block_device
            .read(byte_offset, buffer)
            .expect("could not read from block device");
        assert!(bytes_read == buffer.len(), "short read from block device");
    }

    fn write_bytes(&self, byte_offset: usize, buffer: &[u8]) {
        let mut block_device = self.block_device.lock();
        let bytes_written = block_device
            .write(byte_offset, buffer)
            .expect("could not write to block device");
        assert!(bytes_written == buffer.len(), "short write to block device");
    }

    fn read_value<T>(&self, byte_offset: usize) -> T
    where
        T: FromBytes + Immutable + KnownLayout,
    {
        let mut buffer = alloc::vec![0u8; core::mem::size_of::<T>()];
        self.read_bytes(byte_offset, &mut buffer);
        let (value, _) = T::read_from_prefix(&buffer).unwrap();
        value
    }

    fn write_value<T>(&self, byte_offset: usize, value: &T)
    where
        T: IntoBytes + Immutable + KnownLayout,
    {
        let mut buffer = alloc::vec![0u8; core::mem::size_of::<T>()];
        value.write_to_prefix(&mut buffer).unwrap();
        self.write_bytes(byte_offset, &buffer);
    }

    fn read_u32(&self, byte_offset: usize) -> u32 {
        let mut buffer = [0u8; core::mem::size_of::<u32>()];
        self.read_bytes(byte_offset, &mut buffer);
        let (value, _) = u32::read_from_prefix(&buffer).unwrap();
        value
    }

    fn write_u32(&self, byte_offset: usize, value: u32) {
        let mut buffer = [0u8; core::mem::size_of::<u32>()];
        value.write_to_prefix(&mut buffer).unwrap();
        self.write_bytes(byte_offset, &buffer);
    }

    fn block_group_descriptor_offset(&self, block_group: usize) -> usize {
        const BGD_SIZE: usize = 32;
        let descriptors_per_block = self.block_size / BGD_SIZE;
        let bgd_block =
            (self.superblock.first_data_block as usize) + 1 + block_group / descriptors_per_block;
        self.block_offset(bgd_block) + (block_group % descriptors_per_block) * BGD_SIZE
    }

    fn inode_offset(&self, inumber: u32) -> usize {
        assert!(inumber > 0);
        let block_group = (inumber - 1) / self.superblock.inodes_per_group;
        let (bgd, _) = self.get_block_group_descriptor(block_group as usize);
        let inode_index = ((inumber - 1) % self.superblock.inodes_per_group) as usize;
        self.block_offset(bgd.inode_table as usize)
            + inode_index * (self.superblock.inode_size as usize)
    }

    fn alloc_block(self: &Arc<Self>, preferred_group: usize) -> Option<usize> {
        let _guard = self.block_map_lock.lock();
        let mut bitmap_buffer = alloc::vec![0u8; self.block_size];

        if let Some(b) = self.alloc_block_given_group(preferred_group, &mut bitmap_buffer) {
            return Some(b);
        }

        let groups = (self.superblock.blocks_count / self.superblock.blocks_per_group) as usize;
        for group in 0..groups {
            if group == preferred_group {
                continue;
            }
            if let Some(b) = self.alloc_block_given_group(group, &mut bitmap_buffer) {
                return Some(b);
            }
        }
        None
    }

    fn alloc_block_given_group(
        self: &Arc<Self>,
        group: usize,
        bitmap_buffer: &mut [u8],
    ) -> Option<usize> {
        // Currently this is not threadsafe
        // Someone can get the same block allocated if they call alloc_block_given_group at the same time with the same group, but this is not a problem for our current use case since we only call alloc_block_given_group from alloc_block which has a lock around it
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, bgd_offset) = self.get_block_group_descriptor(group);
            if bgd.free_blocks_count == 0 {
                return None;
            }
            let bgd = BlockGroupDescriptor {
                free_blocks_count: bgd.free_blocks_count - 1,
                ..bgd
            };
            self.write_value(bgd_offset, &bgd);
            bgd.block_bitmap as usize
        };

        self.read_block(bitmap, bitmap_buffer);
        if let Some(i) = self.first_zero_in_bitmap(bitmap_buffer) {
            bitmap_buffer[i / 8] |= 1 << (i % 8);
            self.write_block(bitmap, bitmap_buffer);
            return Some(
                (self.superblock.first_data_block as usize)
                    + i
                    + group * (self.superblock.blocks_per_group as usize),
            );
        }
        panic!("this should never happen");
    }

    pub fn alloc_inode(
        self: &Arc<Self>,
        preferred_group: usize,
        _scratch_buffer: Option<&mut [u8]>,
    ) -> Option<usize> {
        let _guard = self.inode_map_lock.lock();
        let mut bitmap_buffer = alloc::vec![0u8; self.block_size];

        if let Some(b) = self.alloc_inode_given_group(preferred_group, &mut bitmap_buffer) {
            return Some(b);
        }

        let groups = (self.superblock.inodes_count / self.superblock.inodes_per_group) as usize;
        for group in 0..groups {
            if group == preferred_group {
                continue;
            }
            if let Some(b) = self.alloc_inode_given_group(group, &mut bitmap_buffer) {
                return Some(b);
            }
        }
        None
    }

    fn alloc_inode_given_group(
        self: &Arc<Self>,
        group: usize,
        bitmap_buffer: &mut [u8],
    ) -> Option<usize> {
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, bgd_offset) = self.get_block_group_descriptor(group);
            if bgd.free_inodes_count == 0 {
                return None;
            }
            let bgd = BlockGroupDescriptor {
                free_inodes_count: bgd.free_inodes_count - 1,
                ..bgd
            };
            self.write_value(bgd_offset, &bgd);
            bgd.inode_bitmap as usize
        };
        self.read_block(bitmap, bitmap_buffer);
        if let Some(i) = self.first_zero_in_bitmap(bitmap_buffer) {
            bitmap_buffer[i / 8] |= 1 << (i % 8);
            self.write_block(bitmap, bitmap_buffer);
            return Some(1 + i + group * (self.superblock.inodes_per_group as usize));
        }
        panic!("this should never happen");
    }

    pub fn dealloc_block(
        self: &Arc<Self>,
        block_number: usize,
        _scratch_buffer: Option<&mut [u8]>,
    ) {
        let _guard = self.block_map_lock.lock();
        let mut bitmap_buffer = alloc::vec![0u8; self.block_size];
        let block_number = block_number - (self.superblock.first_data_block as usize);
        let group = block_number / (self.superblock.blocks_per_group as usize);
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, bgd_offset) = self.get_block_group_descriptor(group);
            let bgd = BlockGroupDescriptor {
                free_blocks_count: bgd.free_blocks_count + 1,
                ..bgd
            };
            self.write_value(bgd_offset, &bgd);
            bgd.block_bitmap as usize
        };
        self.read_block(bitmap, &mut bitmap_buffer);
        let i = block_number % (self.superblock.blocks_per_group as usize);
        assert!(bitmap_buffer[i / 8] & (1 << (i % 8)) != 0);
        bitmap_buffer[i / 8] &= 0xff ^ (1 << (i % 8));
        self.write_block(bitmap, &bitmap_buffer);
    }

    pub fn dealloc_inode(self: &Arc<Self>, inumber: usize, _scratch_buffer: Option<&mut [u8]>) {
        let _guard = self.inode_map_lock.lock();
        let mut bitmap_buffer = alloc::vec![0u8; self.block_size];
        let inumber = inumber - 1;
        let group = inumber / (self.superblock.inodes_per_group as usize);
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, bgd_offset) = self.get_block_group_descriptor(group);
            let bgd = BlockGroupDescriptor {
                free_inodes_count: bgd.free_inodes_count + 1,
                ..bgd
            };
            self.write_value(bgd_offset, &bgd);
            bgd.inode_bitmap as usize
        };
        self.read_block(bitmap, &mut bitmap_buffer);
        let i = inumber % (self.superblock.inodes_per_group as usize);
        assert!(bitmap_buffer[i / 8] & (1 << (i % 8)) != 0);
        bitmap_buffer[i / 8] &= 0xff ^ (1 << (i % 8));
        self.write_block(bitmap, &bitmap_buffer);
    }

    fn first_zero_in_bitmap(&self, bitmap: &[u8]) -> Option<usize> {
        for i in 0..self.block_size / 8 {
            let (chunk, _) = u64::read_from_prefix(&bitmap[i * 8..]).unwrap();
            if chunk == u64::MAX {
                continue;
            }
            for j in 0usize..8 {
                let mut byte = bitmap[i * 8 + j];
                if byte == 0xff {
                    continue;
                }
                for k in 0usize..8 {
                    if byte & 1 == 0 {
                        return Some(k + 8 * j + 64 * i);
                    }
                    byte >>= 1;
                }
            }
            panic!("at least one 0 should exist");
        }
        None
    }

    fn get_block_group_descriptor(&self, block_group: usize) -> (BlockGroupDescriptor, usize) {
        let bgd_offset = self.block_group_descriptor_offset(block_group);
        (self.read_value(bgd_offset), bgd_offset)
    }

    fn get_inode(self: &Arc<Self>, inumber: u32) -> INode
    where
        Self: Sized,
    {
        let inode_data = self.read_value(self.inode_offset(inumber));
        INode {
            number: inumber as usize,
            data: inode_data,
            dirty: false,
        }
    }

    fn get_fnode(self: &Arc<Self>, inumber: u32) -> Weak<FNode<B>> {
        let mut fnode_cache = self.fnode_cache.lock();
        if let Some(s) = fnode_cache.get(&inumber) {
            return Arc::downgrade(s);
        }
        let inode = self.get_inode(inumber);
        let node = Arc::new(FNode {
            fs: self.clone(),
            inode: IntMutex::new(inode),
        });
        fnode_cache.insert(inumber, node.clone());
        Arc::downgrade(&node)
    }

    pub fn get_root(self: &Arc<Self>) -> Weak<FNode<B>>
    where
        Self: Sized,
    {
        self.get_fnode(2)
    }

    pub fn new(mut block_device: B) -> Result<Self, &'static str> {
        let device_block_size = block_device.block_size();
        if device_block_size < 512 {
            return Err("block size not big enough");
        }

        // get the superblock
        const SUPERBLOCK_START: usize = 1024;
        let mut buffer = [0u8; core::mem::size_of::<Superblock>()];
        let bytes_read = block_device
            .read(SUPERBLOCK_START, &mut buffer)
            .map_err(|_| "could not read superblock")?;
        if bytes_read != buffer.len() {
            return Err("could not read superblock");
        }
        let (superblock, _) =
            Superblock::read_from_prefix(&buffer).map_err(|_| "could not parse superblock")?;

        // safety checks
        if superblock.magic != 0xEF53 {
            return Err("is not a valid ext2 file system");
        }
        if superblock.log_block_size > 2 {
            return Err("invalid block size");
        }
        if superblock.rev_level != 1 {
            return Err("this version of ext2 is too old");
        }

        let block_size = 1024 << superblock.log_block_size;

        // TODO: mark superblock as dirty
        Ok(Self {
            block_size,
            block_device: IntMutex::new(block_device),
            superblock,
            fnode_cache: IntMutex::new(BTreeMap::new()),
            block_map_lock: IntMutex::new(()),
            inode_map_lock: IntMutex::new(()),
            group_lock: IntMutex::new(()),
        })
    }

    fn read_block(self: &Arc<Self>, block_number: usize, buffer: &mut [u8]) {
        assert!(buffer.len() >= self.block_size);
        self.read_bytes(
            self.block_offset(block_number),
            &mut buffer[..self.block_size],
        );
    }

    fn write_block(self: &Arc<Self>, block_number: usize, buffer: &[u8]) {
        assert!(buffer.len() >= self.block_size);
        self.write_bytes(self.block_offset(block_number), &buffer[..self.block_size]);
    }
}

impl<D: Disk> Ext2_old<D> {
    fn alloc_block(
        self: &Arc<Self>,
        preferred_group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Option<usize> {
        let _guard = self.block_map_lock.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };

        if let Some(b) = self.alloc_block_given_group(preferred_group, Some(scratch_buffer)) {
            return Some(b);
        }

        let groups = (self.superblock.blocks_count / self.superblock.blocks_per_group) as usize;
        for group in 0..groups {
            if group == preferred_group {
                continue;
            }
            if let Some(b) = self.alloc_block_given_group(group, Some(scratch_buffer)) {
                return Some(b);
            }
        }
        None
    }

    fn alloc_block_given_group(
        self: &Arc<Self>,
        group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Option<usize> {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, (bgd_offset, bgd_block)) =
                self.get_block_group_descriptor(group, Some(scratch_buffer));
            if bgd.free_blocks_count == 0 {
                return None;
            }
            let bgd = BlockGroupDescriptor {
                free_blocks_count: bgd.free_blocks_count - 1,
                ..bgd
            };
            bgd.write_to_prefix(&mut scratch_buffer[bgd_offset..])
                .unwrap();
            self.write_block(bgd_block, scratch_buffer);
            bgd.block_bitmap as usize
        };
        self.read_block(bitmap, scratch_buffer);
        if let Some(i) = self.first_zero_in_bitmap(scratch_buffer) {
            scratch_buffer[i / 8] |= 1 << (i % 8);
            self.write_block(bitmap, scratch_buffer);
            return Some(
                (self.superblock.first_data_block as usize)
                    + i
                    + group * (self.superblock.blocks_per_group as usize),
            );
        }
        panic!("this should never happen");
    }

    pub fn alloc_inode(
        self: &Arc<Self>,
        preferred_group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Option<usize> {
        let _guard = self.inode_map_lock.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };

        if let Some(b) = self.alloc_inode_given_group(preferred_group, Some(scratch_buffer)) {
            return Some(b);
        }

        let groups = (self.superblock.inodes_count / self.superblock.inodes_per_group) as usize;
        for group in 0..groups {
            if group == preferred_group {
                continue;
            }
            if let Some(b) = self.alloc_inode_given_group(group, Some(scratch_buffer)) {
                return Some(b);
            }
        }
        None
    }

    fn alloc_inode_given_group(
        self: &Arc<Self>,
        group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Option<usize> {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, (bgd_offset, bgd_block)) =
                self.get_block_group_descriptor(group, Some(scratch_buffer));
            if bgd.free_inodes_count == 0 {
                return None;
            }
            let bgd = BlockGroupDescriptor {
                free_inodes_count: bgd.free_inodes_count - 1,
                ..bgd
            };
            bgd.write_to_prefix(&mut scratch_buffer[bgd_offset..])
                .unwrap();
            self.write_block(bgd_block, scratch_buffer);
            bgd.inode_bitmap as usize
        };
        self.read_block(bitmap, scratch_buffer);
        if let Some(i) = self.first_zero_in_bitmap(scratch_buffer) {
            scratch_buffer[i / 8] |= 1 << (i % 8);
            self.write_block(bitmap, scratch_buffer);
            return Some(1 + i + group * (self.superblock.inodes_per_group as usize));
        }
        panic!("this should never happen");
    }

    pub fn dealloc_block(self: &Arc<Self>, block_number: usize, scratch_buffer: Option<&mut [u8]>) {
        let _guard = self.block_map_lock.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        let block_number = block_number - (self.superblock.first_data_block as usize);
        let group = block_number / (self.superblock.blocks_per_group as usize);
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, (bgd_offset, bgd_block)) =
                self.get_block_group_descriptor(group, Some(scratch_buffer));
            let bgd = BlockGroupDescriptor {
                free_blocks_count: bgd.free_blocks_count + 1,
                ..bgd
            };
            bgd.write_to_prefix(&mut scratch_buffer[bgd_offset..])
                .unwrap();
            self.write_block(bgd_block, scratch_buffer);
            bgd.block_bitmap as usize
        };
        self.read_block(bitmap, scratch_buffer);
        let i = block_number % (self.superblock.blocks_per_group as usize);
        assert!(scratch_buffer[i / 8] & (1 << (i % 8)) != 0);
        scratch_buffer[i / 8] &= 0xff ^ (1 << (i % 8));
        self.write_block(bitmap, scratch_buffer);
    }

    pub fn dealloc_inode(self: &Arc<Self>, inumber: usize, scratch_buffer: Option<&mut [u8]>) {
        let _guard = self.inode_map_lock.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        let inumber = inumber - 1;
        let group = inumber / (self.superblock.inodes_per_group as usize);
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, (bgd_offset, bgd_block)) =
                self.get_block_group_descriptor(group, Some(scratch_buffer));
            let bgd = BlockGroupDescriptor {
                free_inodes_count: bgd.free_inodes_count + 1,
                ..bgd
            };
            bgd.write_to_prefix(&mut scratch_buffer[bgd_offset..])
                .unwrap();
            self.write_block(bgd_block, scratch_buffer);
            bgd.inode_bitmap as usize
        };
        self.read_block(bitmap, scratch_buffer);
        let i = inumber % (self.superblock.inodes_per_group as usize);
        assert!(scratch_buffer[i / 8] & (1 << (i % 8)) != 0);
        scratch_buffer[i / 8] &= 0xff ^ (1 << (i % 8));
        self.write_block(bitmap, scratch_buffer);
    }

    fn first_zero_in_bitmap(self: &Arc<Self>, bitmap: &[u8]) -> Option<usize> {
        for i in 0..self.block_size / 8 {
            let (chunk, _) = u64::read_from_prefix(&bitmap[i * 8..]).unwrap();
            if chunk == u64::MAX {
                continue;
            }
            for j in 0usize..8 {
                let mut byte = bitmap[i * 8 + j];
                if byte == 0xff {
                    continue;
                }
                for k in 0usize..8 {
                    if byte & 1 == 0 {
                        return Some(k + 8 * j + 64 * i);
                    }
                    byte >>= 1;
                }
            }
            panic!("at least one 0 should exist");
        }
        None
    }

    fn get_block_group_descriptor(
        self: &Arc<Self>,
        block_group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> (BlockGroupDescriptor, (usize, usize)) {
        const BGD_SIZE: usize = 32;
        let descriptors_per_block = self.block_size / BGD_SIZE;
        let bgd_block =
            (self.superblock.first_data_block as usize) + 1 + block_group / descriptors_per_block;
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        self.read_block(bgd_block, scratch_buffer);
        let bgd_index = block_group % descriptors_per_block;
        let bgd_offset = bgd_index * BGD_SIZE;
        let (bgd, _) =
            BlockGroupDescriptor::read_from_prefix(&scratch_buffer[bgd_offset..]).unwrap();
        (bgd, (bgd_offset, bgd_block))
    }

    fn get_inode(
        self: &Arc<Self>,
        inumber: u32,
        scratch_buffer: Option<&mut [u8]>,
    ) -> (INode, (usize, usize))
    where
        Self: Sized,
    {
        assert!(inumber > 0);
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        let block_group = (inumber - 1) / self.superblock.inodes_per_group;
        let _guard = self.group_lock.lock();
        let (bgd, _) = self.get_block_group_descriptor(block_group as usize, Some(scratch_buffer));
        let inodes_per_block = self.block_size / (self.superblock.inode_size as usize);
        let inode_index = ((inumber - 1) % self.superblock.inodes_per_group) as usize;
        let inode_block = (bgd.inode_table as usize) + inode_index / inodes_per_block;
        self.read_block(inode_block, scratch_buffer);
        let inode_index = inode_index % inodes_per_block;
        let inode_offset = inode_index * (self.superblock.inode_size as usize);
        let (inode_data, _) = INodeData::read_from_prefix(&scratch_buffer[inode_offset..]).unwrap();
        (
            INode {
                number: inumber as usize,
                data: inode_data,
                dirty: false,
            },
            (inode_block, inode_offset),
        )
    }

    pub fn get_root(self: &Arc<Self>) -> Weak<FNode_old<D>>
    where
        Self: Sized,
    {
        self.get_fnode(2, None)
    }

    fn get_fnode(
        self: &Arc<Self>,
        inumber: u32,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Weak<FNode_old<D>> {
        let mut fnode_cache = self.fnode_cache.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        if let Some(s) = fnode_cache.get(&inumber) {
            return Arc::downgrade(s);
        }
        let (inode, _) = self.get_inode(inumber, Some(scratch_buffer));
        let node = Arc::new(FNode_old {
            fs: self.clone(),
            inode: IntMutex::new(inode),
        });
        fnode_cache.insert(inumber, node.clone());
        Arc::downgrade(&node)
    }

    pub fn new(disk: D) -> Result<Self, &'static str> {
        if disk.sector_size() < 512 {
            return Err("sector size not big enough");
        }

        // get the superblock
        const SUPERBLOCK_START: usize = 1024;
        let superblock_sector = SUPERBLOCK_START / disk.sector_size();
        let superblock_offset = SUPERBLOCK_START % disk.sector_size();
        let mut buffer = alloc::vec![0u8; disk.sector_size()];
        disk.read_sector(superblock_sector, &mut buffer);
        let (superblock, _) = Superblock::read_from_prefix(&buffer[superblock_offset..])
            .map_err(|_| "could not parse superblock")?;

        // safety checks
        if superblock.magic != 0xEF53 {
            return Err("is not a valid ext2 file system");
        }
        if superblock.log_block_size > 2 {
            return Err("invalid block size");
        }
        if superblock.rev_level != 1 {
            return Err("this version of ext2 is too old");
        }

        // TODO: mark superblock as dirty
        Ok(Self {
            block_size: 1024 << superblock.log_block_size,
            disk: IntMutex::new(disk),
            superblock,
            fnode_cache: IntMutex::new(BTreeMap::new()),
            block_map_lock: IntMutex::new(()),
            inode_map_lock: IntMutex::new(()),
            group_lock: IntMutex::new(()),
        })
    }

    fn read_block(self: &Arc<Self>, block_number: usize, buffer: &mut [u8]) {
        let disk = self.disk.lock();
        let sector_size = disk.sector_size();
        let factor = self.block_size / sector_size;
        for i in 0..factor {
            let start = i * sector_size;
            let end = start + sector_size;
            disk.read_sector(block_number * factor + i, &mut buffer[start..end]);
        }
    }

    fn write_block(self: &Arc<Self>, block_number: usize, buffer: &[u8]) {
        let mut disk = self.disk.lock();
        let sector_size = disk.sector_size();
        let factor = self.block_size / sector_size;
        for i in 0..factor {
            let start = i * sector_size;
            let end = start + sector_size;
            disk.write_sector(block_number * factor + i, &buffer[start..end]);
        }
    }
}

impl<D: Disk> FNode_old<D> {
    fn block_tree(
        self: &Arc<Self>,
        block_number: usize,
        scratch_buffer: Option<&mut [u8]>,
        inode: &INode,
    ) -> ([usize; 4], [usize; 4], usize) {
        let mut list = [0usize; 4];
        let (indices, depth) = self.block_tree_indices(block_number);
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.fs.block_size])[..],
        };

        list[0] = inode.data.block[indices[0]] as usize;
        for i in 1..depth {
            if list[i - 1] == 0 {
                return (list, indices, depth);
            }
            self.fs.read_block(list[i - 1], scratch_buffer);
            let index = indices[i];
            let start = index * 4;
            let end = start + 4;
            let (next_bn, _) = u32::read_from_prefix(&scratch_buffer[start..end]).unwrap();
            list[i] = next_bn as usize;
        }
        (list, indices, depth)
    }

    fn block_tree_indices(self: &Arc<Self>, block_number: usize) -> ([usize; 4], usize) {
        if block_number < 12 {
            return ([block_number, 0, 0, 0], 1);
        }
        // blocks per block
        let bpb = self.fs.block_size / 4;

        // one level indirection
        let block_number = block_number - 12;
        if block_number < bpb {
            return ([12, block_number, 0, 0], 2);
        }

        // two level indirection
        let block_number = block_number - bpb;
        if block_number < bpb * bpb {
            return ([13, block_number / bpb, block_number % bpb, 0], 3);
        }

        // three level indirection
        let block_number = block_number - bpb * bpb;
        (
            [
                14,
                block_number / (bpb * bpb),
                block_number / bpb % bpb,
                block_number % (bpb * bpb),
            ],
            4,
        )
    }

    pub fn read_block(self: &Arc<Self>, block_number: usize, buffer: &mut [u8], inode: &INode) {
        let (tree, _, size) = self.block_tree(block_number, Some(buffer), inode);
        match tree[size - 1] {
            0 => buffer[0..self.fs.block_size].fill(0),
            b => self.fs.read_block(b, buffer),
        }
    }

    pub fn write_block(
        self: &Arc<Self>,
        block_number: usize,
        buffer: &[u8],
        inode: &mut INode,
        scratch_buffer: Option<&mut [u8]>,
        new_size: Option<usize>,
        preferred_group: usize,
    ) {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.fs.block_size])[..],
        };
        let (mut tree, indices, size) = self.block_tree(block_number, Some(scratch_buffer), inode);
        for i in 0..size {
            if !(tree[i] == 0 || (i == 0 && new_size.is_some())) {
                continue;
            }

            tree[i] = self
                .fs
                .alloc_block(preferred_group, Some(scratch_buffer))
                .unwrap();
            if i == 0 {
                let new_size = match new_size {
                    Some(s) => s as u32,
                    None => inode.data.size,
                };
                let mut new_array = inode.data.block;
                new_array[indices[i]] = tree[i] as u32;
                let new_inode = INode {
                    dirty: true,
                    number: inode.number,
                    data: INodeData {
                        size: new_size,
                        block: new_array,
                        ..inode.data
                    },
                };
                self.update_inode(inode, new_inode, Some(scratch_buffer));
            } else {
                scratch_buffer[0..self.fs.block_size].fill(0);
                (tree[i] as u32)
                    .write_to_prefix(&mut scratch_buffer[indices[i] * 4..])
                    .unwrap();
                self.fs.write_block(tree[i - 1], scratch_buffer);
            }
        }
        self.fs.write_block(tree[size - 1], buffer);
    }

    fn update_inode(
        self: &Arc<Self>,
        old: &mut INode,
        new: INode,
        scratch_buffer: Option<&mut [u8]>,
    ) {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.fs.block_size])[..],
        };
        let (_, (inode_block, inode_offset)) =
            self.fs.get_inode(old.number as u32, Some(scratch_buffer));
        new.data
            .write_to_prefix(&mut scratch_buffer[inode_offset..])
            .unwrap();
        self.fs.write_block(inode_block, scratch_buffer);
        *old = new;
    }

    // use indexing
    pub fn create_entry(
        self: &Arc<Self>,
        entry_name: &str,
        inumber: u32,
        file_type: u8,
    ) -> Result<(), &'static str> {
        assert!(inumber > 0);
        let mut inode = self.inode.lock();
        // TODO proper types
        assert!(inode.data.mode & 0xF000 == 0x4000);
        let mut pointer: usize = 0;
        let mut last_fetched_block: usize = 1;
        let mut buffer = alloc::vec![0u8; self.fs.block_size];
        let mut placement = None;
        let entry_space = |name: &str| (8 + name.len()).next_multiple_of(4);
        let needed = entry_space(entry_name);
        while pointer < (inode.data.size as usize) {
            assert!(pointer.is_multiple_of(4));
            let needed_block = pointer / self.fs.block_size;
            if needed_block != last_fetched_block {
                self.read_block(needed_block, &mut buffer, &inode);
                last_fetched_block = needed_block;
            }
            let offset = pointer % self.fs.block_size;
            let (inumber, _) = u32::read_from_prefix(&buffer[offset..]).unwrap();
            let (rec_len, _) = u16::read_from_prefix(&buffer[offset + 4..]).unwrap();
            assert!(rec_len > 0);
            let (name_len, _) = u8::read_from_prefix(&buffer[offset + 6..]).unwrap();
            if inumber != 0 {
                let name = &buffer[offset + 8..offset + 8 + (name_len as usize)];
                let name = core::str::from_utf8(name).unwrap();
                if name == entry_name {
                    return Err("already exists");
                }
                let actually_needed = entry_space(name);
                if placement.is_none() && (rec_len as usize) >= needed + actually_needed {
                    placement = Some((pointer, actually_needed, rec_len));
                }
            }
            pointer += rec_len as usize;
        }
        let ((placement, wanted_first_size, actual_first_size), new_block) = match placement {
            Some(p) => (p, false),
            None => (
                (inode.data.size as usize, 0, self.fs.block_size as u16),
                true,
            ),
        };
        let bn = (placement + wanted_first_size) / self.fs.block_size;
        if !new_block {
            self.read_block(bn, &mut buffer, &inode);
        } else {
            buffer.fill(0);
        }
        assert!(bn == placement / self.fs.block_size);
        let offset = (placement + wanted_first_size) % self.fs.block_size;
        inumber.write_to_prefix(&mut buffer[offset..]).unwrap();
        (actual_first_size - wanted_first_size as u16)
            .write_to_prefix(&mut buffer[offset + 4..])
            .unwrap();
        (entry_name.len() as u8)
            .write_to_prefix(&mut buffer[offset + 6..])
            .unwrap();
        file_type
            .write_to_prefix(&mut buffer[offset + 7..])
            .unwrap();
        entry_name
            .write_to_prefix(&mut buffer[offset + 8..])
            .unwrap();
        if !new_block {
            let offset = placement % self.fs.block_size;
            assert!(wanted_first_size > 0);
            (wanted_first_size as u16)
                .write_to_prefix(&mut buffer[offset + 4..])
                .unwrap();
        }
        let ideal_group = (inode.number - 1) / (self.fs.superblock.inodes_per_group as usize);
        let new_size = if new_block {
            Some(inode.data.size as usize + self.fs.block_size)
        } else {
            None
        };
        self.write_block(bn, &buffer, &mut inode, None, new_size, ideal_group);
        Ok(())
    }

    // TODO: use indexing instead of linsearch
    pub fn search(self: &Arc<Self>, next: &str) -> Option<Weak<FNode_old<D>>> {
        let inode = self.inode.lock();
        // TODO proper types
        assert!(inode.data.mode & 0xF000 == 0x4000);
        let mut pointer: usize = 0;
        let mut last_fetched_block: usize = 1;
        let mut buffer = alloc::vec![0u8; self.fs.block_size];
        while pointer < (inode.data.size as usize) {
            assert!(pointer.is_multiple_of(4));
            let needed_block = pointer / self.fs.block_size;
            if needed_block != last_fetched_block {
                self.read_block(needed_block, &mut buffer, &inode);
                last_fetched_block = needed_block;
            }
            let offset = pointer % self.fs.block_size;
            let (inumber, _) = u32::read_from_prefix(&buffer[offset..]).unwrap();
            let (rec_len, _) = u16::read_from_prefix(&buffer[offset + 4..]).unwrap();
            let (name_len, _) = u8::read_from_prefix(&buffer[offset + 6..]).unwrap();
            if inumber != 0 {
                let name = &buffer[offset + 8..offset + 8 + (name_len as usize)];
                let name = core::str::from_utf8(name).unwrap();
                if name == next {
                    return Some(self.fs.get_fnode(inumber, Some(&mut buffer[..])));
                }
            }
            pointer += rec_len as usize;
        }
        None
    }
}

impl<B: BlockDevice> FNode<B> {
    fn block_tree(
        self: &Arc<Self>,
        block_number: usize,
        inode: &INode,
    ) -> ([usize; 4], [usize; 4], usize) {
        let mut list = [0usize; 4];
        let (indices, depth) = self.block_tree_indices(block_number);

        list[0] = inode.data.block[indices[0]] as usize;
        for i in 1..depth {
            if list[i - 1] == 0 {
                return (list, indices, depth);
            }
            list[i] = self
                .fs
                .read_u32(self.fs.indirect_block_entry_offset(list[i - 1], indices[i]))
                as usize;
        }
        (list, indices, depth)
    }

    fn block_tree_indices(self: &Arc<Self>, block_number: usize) -> ([usize; 4], usize) {
        if block_number < 12 {
            return ([block_number, 0, 0, 0], 1);
        }
        // blocks per block
        let bpb = self.fs.block_size / 4;

        // one level indirection
        let block_number = block_number - 12;
        if block_number < bpb {
            return ([12, block_number, 0, 0], 2);
        }

        // two level indirection
        let block_number = block_number - bpb;
        if block_number < bpb * bpb {
            return ([13, block_number / bpb, block_number % bpb, 0], 3);
        }

        // three level indirection
        let block_number = block_number - bpb * bpb;
        (
            [
                14,
                block_number / (bpb * bpb),
                block_number / bpb % bpb,
                block_number % (bpb * bpb),
            ],
            4,
        )
    }

    pub fn read_block(self: &Arc<Self>, block_number: usize, buffer: &mut [u8], inode: &INode) {
        let (tree, _, size) = self.block_tree(block_number, inode);
        match tree[size - 1] {
            0 => buffer[0..self.fs.block_size].fill(0),
            b => self.fs.read_block(b, buffer),
        }
    }

    pub fn write_block(
        self: &Arc<Self>,
        block_number: usize,
        buffer: &[u8],
        inode: &mut INode,
        _scratch_buffer: Option<&mut [u8]>,
        new_size: Option<usize>,
        preferred_group: usize,
    ) {
        let (mut tree, indices, size) = self.block_tree(block_number, inode);
        let zero_block = alloc::vec![0u8; self.fs.block_size];
        for i in 0..size {
            if !(tree[i] == 0 || (i == 0 && new_size.is_some())) {
                continue;
            }

            tree[i] = self.fs.alloc_block(preferred_group).unwrap();
            if i + 1 < size {
                self.fs.write_block(tree[i], &zero_block);
            }
            if i == 0 {
                let new_size = match new_size {
                    Some(s) => s as u32,
                    None => inode.data.size,
                };
                let mut new_array = inode.data.block;
                new_array[indices[i]] = tree[i] as u32;
                let new_inode = INode {
                    dirty: true,
                    number: inode.number,
                    data: INodeData {
                        size: new_size,
                        block: new_array,
                        ..inode.data
                    },
                };
                self.update_inode(inode, new_inode);
            }
            if i > 0 {
                self.fs.write_u32(
                    self.fs.indirect_block_entry_offset(tree[i - 1], indices[i]),
                    tree[i] as u32,
                );
            }
        }
        self.fs.write_block(tree[size - 1], buffer);
    }

    fn update_inode(self: &Arc<Self>, old: &mut INode, new: INode) {
        self.fs
            .write_value(self.fs.inode_offset(old.number as u32), &new.data);
        *old = new;
    }

    // use indexing
    pub fn create_entry(
        self: &Arc<Self>,
        entry_name: &str,
        inumber: u32,
        file_type: u8,
    ) -> Result<(), &'static str> {
        assert!(inumber > 0);
        let mut inode = self.inode.lock();
        // TODO proper types
        assert!(inode.data.mode & 0xF000 == 0x4000);
        let mut pointer: usize = 0;
        let mut last_fetched_block: usize = 1;
        let mut buffer = alloc::vec![0u8; self.fs.block_size];
        let mut placement = None;
        let entry_space = |name: &str| (8 + name.len()).next_multiple_of(4);
        let needed = entry_space(entry_name);
        while pointer < (inode.data.size as usize) {
            assert!(pointer.is_multiple_of(4));
            let needed_block = pointer / self.fs.block_size;
            if needed_block != last_fetched_block {
                self.read_block(needed_block, &mut buffer, &inode);
                last_fetched_block = needed_block;
            }
            let offset = pointer % self.fs.block_size;
            let (inumber, _) = u32::read_from_prefix(&buffer[offset..]).unwrap();
            let (rec_len, _) = u16::read_from_prefix(&buffer[offset + 4..]).unwrap();
            assert!(rec_len > 0);
            let (name_len, _) = u8::read_from_prefix(&buffer[offset + 6..]).unwrap();
            if inumber != 0 {
                let name = &buffer[offset + 8..offset + 8 + (name_len as usize)];
                let name = core::str::from_utf8(name).unwrap();
                if name == entry_name {
                    return Err("already exists");
                }
                let actually_needed = entry_space(name);
                if placement.is_none() && (rec_len as usize) >= needed + actually_needed {
                    placement = Some((pointer, actually_needed, rec_len));
                }
            }
            pointer += rec_len as usize;
        }
        let ((placement, wanted_first_size, actual_first_size), new_block) = match placement {
            Some(p) => (p, false),
            None => (
                (inode.data.size as usize, 0, self.fs.block_size as u16),
                true,
            ),
        };
        let bn = (placement + wanted_first_size) / self.fs.block_size;
        if !new_block {
            self.read_block(bn, &mut buffer, &inode);
        } else {
            buffer.fill(0);
        }
        assert!(bn == placement / self.fs.block_size);
        let offset = (placement + wanted_first_size) % self.fs.block_size;
        inumber.write_to_prefix(&mut buffer[offset..]).unwrap();
        (actual_first_size - wanted_first_size as u16)
            .write_to_prefix(&mut buffer[offset + 4..])
            .unwrap();
        (entry_name.len() as u8)
            .write_to_prefix(&mut buffer[offset + 6..])
            .unwrap();
        file_type
            .write_to_prefix(&mut buffer[offset + 7..])
            .unwrap();
        entry_name
            .write_to_prefix(&mut buffer[offset + 8..])
            .unwrap();
        if !new_block {
            let offset = placement % self.fs.block_size;
            assert!(wanted_first_size > 0);
            (wanted_first_size as u16)
                .write_to_prefix(&mut buffer[offset + 4..])
                .unwrap();
        }
        let ideal_group = (inode.number - 1) / (self.fs.superblock.inodes_per_group as usize);
        let new_size = if new_block {
            Some(inode.data.size as usize + self.fs.block_size)
        } else {
            None
        };
        self.write_block(bn, &buffer, &mut inode, None, new_size, ideal_group);
        Ok(())
    }

    // TODO: use indexing instead of linsearch
    pub fn search(self: &Arc<Self>, next: &str) -> Option<Weak<FNode<B>>> {
        let inode = self.inode.lock();
        // TODO proper types
        assert!(inode.data.mode & 0xF000 == 0x4000);
        let mut pointer: usize = 0;
        let mut last_fetched_block: usize = 1;
        let mut buffer = alloc::vec![0u8; self.fs.block_size];
        while pointer < (inode.data.size as usize) {
            assert!(pointer.is_multiple_of(4));
            let needed_block = pointer / self.fs.block_size;
            if needed_block != last_fetched_block {
                self.read_block(needed_block, &mut buffer, &inode);
                last_fetched_block = needed_block;
            }
            let offset = pointer % self.fs.block_size;
            let (inumber, _) = u32::read_from_prefix(&buffer[offset..]).unwrap();
            let (rec_len, _) = u16::read_from_prefix(&buffer[offset + 4..]).unwrap();
            let (name_len, _) = u8::read_from_prefix(&buffer[offset + 6..]).unwrap();
            if inumber != 0 {
                let name = &buffer[offset + 8..offset + 8 + (name_len as usize)];
                let name = core::str::from_utf8(name).unwrap();
                if name == next {
                    return Some(self.fs.get_fnode(inumber));
                }
            }
            pointer += rec_len as usize;
        }
        None
    }
}

#[cfg(test)]
mod test {
    use super::super::ramdisk::Ramdisk;
    use super::Ext2;
    use crate::print::{kprint, kprintln};
    use crate::sync::MutexLike;
    use alloc::string::ToString;
    use alloc::sync::Arc;

    #[test_case]
    fn test_block_allocation() {
        let disk = Ramdisk::new(512);
        let fs = Arc::new(Ext2::new(disk).unwrap());
        let root = fs.get_root().upgrade().unwrap();
        let hello = root.search("hello").unwrap().upgrade().unwrap();
        let mut buffer = alloc::vec![0u8; fs.block_size];
        {
            let mut inode = hello.inode.lock();
            hello.read_block(0, &mut buffer, &inode);
            match core::str::from_utf8(&buffer[..]) {
                Ok(s) => {
                    let mut iter = s.chars();
                    while let Some(c) = iter.next()
                        && c != '\0'
                    {
                        kprint!("{}", c)
                    }
                    kprintln!("");
                }
                Err(g) => kprintln!("dead {}", g),
            };
            buffer[0] = b'b';
            hello.write_block(
                999,
                &buffer,
                &mut inode,
                None,
                Some(fs.block_size * 1000),
                0,
            );
            kprintln!("trying to read now");
            hello.read_block(999, &mut buffer, &inode);
            match core::str::from_utf8(&buffer[..]) {
                Ok(s) => {
                    let mut iter = s.chars();
                    while let Some(c) = iter.next()
                        && c != '\0'
                    {
                        kprint!("{}", c)
                    }
                    kprintln!("");
                }
                Err(g) => kprintln!("dead {}", g),
            };
        }
        // kprintln!("{}", fs.alloc_block(0, None).unwrap());
    }

    #[test_case]
    fn test_create_entries() {
        let disk = Ramdisk::new(512);
        let fs = Arc::new(Ext2::new(disk).unwrap());
        let root = fs.get_root().upgrade().unwrap();
        for i in 0i32..100 {
            kprintln!("WRITING: {}", i);
            let _ = root.create_entry(&i.to_string(), 1, 0);
        }

        for i in 0i32..100 {
            kprintln!("READING: {}", i);
            assert!(root.search(&i.to_string()).is_some());
        }
    }
}
