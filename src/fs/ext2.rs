extern crate alloc;

use crate::devices::block::BlockDevice;
use crate::sync::IntMutex;
use crate::sync::MutexLike;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::{collections::btree_map::BTreeMap, sync::Arc, sync::Weak};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

pub struct Ext2 {
    block_size: usize,
    block_device: IntMutex<Box<dyn BlockDevice + Send + Sync>>,
    superblock: Superblock,
    fnode_cache: IntMutex<BTreeMap<u32, Arc<FNode>>>,
    block_map_lock: IntMutex<()>,
    inode_map_lock: IntMutex<()>,
    group_lock: IntMutex<()>,
}

pub struct FNode {
    fs: Arc<Ext2>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    AlreadyExists,
    NoSpace,
    WriteError,
    ReadError,
    InvalidInput,
    Corrupted(String),
    Other(String),
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

impl Ext2 {
    fn alloc_block(
        self: &Arc<Self>,
        preferred_group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<usize, FsError> {
        let _guard = self.block_map_lock.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };

        if let Some(b) = self.alloc_block_given_group(preferred_group, Some(scratch_buffer))? {
            return Ok(b);
        }

        let groups = (self.superblock.blocks_count / self.superblock.blocks_per_group) as usize;
        for group in 0..groups {
            if group == preferred_group {
                continue;
            }
            if let Some(b) = self.alloc_block_given_group(group, Some(scratch_buffer))? {
                return Ok(b);
            }
        }
        Err(FsError::NoSpace)
    }

    fn alloc_block_given_group(
        self: &Arc<Self>,
        group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<Option<usize>, FsError> {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, (bgd_offset, bgd_block)) =
                self.get_block_group_descriptor(group, Some(scratch_buffer))?;
            if bgd.free_blocks_count == 0 {
                return Ok(None);
            }
            let bgd = BlockGroupDescriptor {
                free_blocks_count: bgd.free_blocks_count - 1,
                ..bgd
            };
            bgd.write_to_prefix(&mut scratch_buffer[bgd_offset..])
                .unwrap();
            self.write_block(bgd_block, scratch_buffer)?;
            bgd.block_bitmap as usize
        };
        self.read_block(bitmap, scratch_buffer)?;
        if let Some(i) = self.first_zero_in_bitmap(scratch_buffer) {
            scratch_buffer[i / 8] |= 1 << (i % 8);
            self.write_block(bitmap, scratch_buffer)?;
            return Ok(Some(
                (self.superblock.first_data_block as usize)
                    + i
                    + group * (self.superblock.blocks_per_group as usize),
            ));
        }
        Err(FsError::Corrupted(
            "block bitmap reports free entries but none were found".into(),
        ))
    }

    pub fn alloc_inode(
        self: &Arc<Self>,
        preferred_group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<usize, FsError> {
        let _guard = self.inode_map_lock.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };

        if let Some(b) = self.alloc_inode_given_group(preferred_group, Some(scratch_buffer))? {
            return Ok(b);
        }

        let groups = (self.superblock.inodes_count / self.superblock.inodes_per_group) as usize;
        for group in 0..groups {
            if group == preferred_group {
                continue;
            }
            if let Some(b) = self.alloc_inode_given_group(group, Some(scratch_buffer))? {
                return Ok(b);
            }
        }
        Err(FsError::NoSpace)
    }

    fn alloc_inode_given_group(
        self: &Arc<Self>,
        group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<Option<usize>, FsError> {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        let bitmap = {
            let _guard = self.group_lock.lock();
            let (bgd, (bgd_offset, bgd_block)) =
                self.get_block_group_descriptor(group, Some(scratch_buffer))?;
            if bgd.free_inodes_count == 0 {
                return Ok(None);
            }
            let bgd = BlockGroupDescriptor {
                free_inodes_count: bgd.free_inodes_count - 1,
                ..bgd
            };
            bgd.write_to_prefix(&mut scratch_buffer[bgd_offset..])
                .unwrap();
            self.write_block(bgd_block, scratch_buffer)?;
            bgd.inode_bitmap as usize
        };
        self.read_block(bitmap, scratch_buffer)?;
        if let Some(i) = self.first_zero_in_bitmap(scratch_buffer) {
            scratch_buffer[i / 8] |= 1 << (i % 8);
            self.write_block(bitmap, scratch_buffer)?;
            return Ok(Some(
                1 + i + group * (self.superblock.inodes_per_group as usize),
            ));
        }
        Err(FsError::Corrupted(
            "inode bitmap reports free entries but none were found".into(),
        ))
    }

    pub fn dealloc_block(
        self: &Arc<Self>,
        block_number: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<(), FsError> {
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
                self.get_block_group_descriptor(group, Some(scratch_buffer))?;
            let bgd = BlockGroupDescriptor {
                free_blocks_count: bgd.free_blocks_count + 1,
                ..bgd
            };
            bgd.write_to_prefix(&mut scratch_buffer[bgd_offset..])
                .unwrap();
            self.write_block(bgd_block, scratch_buffer)?;
            bgd.block_bitmap as usize
        };
        self.read_block(bitmap, scratch_buffer)?;
        let i = block_number % (self.superblock.blocks_per_group as usize);
        assert!(scratch_buffer[i / 8] & (1 << (i % 8)) != 0);
        scratch_buffer[i / 8] &= 0xff ^ (1 << (i % 8));
        self.write_block(bitmap, scratch_buffer)?;
        Ok(())
    }

    pub fn dealloc_inode(
        self: &Arc<Self>,
        inumber: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<(), FsError> {
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
                self.get_block_group_descriptor(group, Some(scratch_buffer))?;
            let bgd = BlockGroupDescriptor {
                free_inodes_count: bgd.free_inodes_count + 1,
                ..bgd
            };
            bgd.write_to_prefix(&mut scratch_buffer[bgd_offset..])
                .unwrap();
            self.write_block(bgd_block, scratch_buffer)?;
            bgd.inode_bitmap as usize
        };
        self.read_block(bitmap, scratch_buffer)?;
        let i = inumber % (self.superblock.inodes_per_group as usize);
        assert!(scratch_buffer[i / 8] & (1 << (i % 8)) != 0);
        scratch_buffer[i / 8] &= 0xff ^ (1 << (i % 8));
        self.write_block(bitmap, scratch_buffer)?;
        Ok(())
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
        }
        None
    }

    fn get_block_group_descriptor(
        self: &Arc<Self>,
        block_group: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<(BlockGroupDescriptor, (usize, usize)), FsError> {
        const BGD_SIZE: usize = 32;
        let descriptors_per_block = self.block_size / BGD_SIZE;
        let bgd_block =
            (self.superblock.first_data_block as usize) + 1 + block_group / descriptors_per_block;
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        self.read_block(bgd_block, scratch_buffer)?;
        let bgd_index = block_group % descriptors_per_block;
        let bgd_offset = bgd_index * BGD_SIZE;
        let (bgd, _) =
            BlockGroupDescriptor::read_from_prefix(&scratch_buffer[bgd_offset..]).unwrap();
        Ok((bgd, (bgd_offset, bgd_block)))
    }

    fn get_inode(
        self: &Arc<Self>,
        inumber: u32,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<(INode, (usize, usize)), FsError>
    where
        Self: Sized,
    {
        if inumber == 0 {
            return Err(FsError::InvalidInput);
        }
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        let block_group = (inumber - 1) / self.superblock.inodes_per_group;
        let _guard = self.group_lock.lock();
        let (bgd, _) =
            self.get_block_group_descriptor(block_group as usize, Some(scratch_buffer))?;
        let inodes_per_block = self.block_size / (self.superblock.inode_size as usize);
        let inode_index = ((inumber - 1) % self.superblock.inodes_per_group) as usize;
        let inode_block = (bgd.inode_table as usize) + inode_index / inodes_per_block;
        self.read_block(inode_block, scratch_buffer)?;
        let inode_index = inode_index % inodes_per_block;
        let inode_offset = inode_index * (self.superblock.inode_size as usize);
        let (inode_data, _) = INodeData::read_from_prefix(&scratch_buffer[inode_offset..]).unwrap();
        Ok((
            INode {
                number: inumber as usize,
                data: inode_data,
                dirty: false,
            },
            (inode_block, inode_offset),
        ))
    }

    pub fn get_root(self: &Arc<Self>) -> Result<Weak<FNode>, FsError>
    where
        Self: Sized,
    {
        self.get_fnode(2, None)
    }

    fn get_fnode(
        self: &Arc<Self>,
        inumber: u32,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<Weak<FNode>, FsError> {
        let mut fnode_cache = self.fnode_cache.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        if let Some(s) = fnode_cache.get(&inumber) {
            return Ok(Arc::downgrade(s));
        }
        let (inode, _) = self.get_inode(inumber, Some(scratch_buffer))?;
        let node = Arc::new(FNode {
            fs: self.clone(),
            inode: IntMutex::new(inode),
        });
        fnode_cache.insert(inumber, node.clone());
        Ok(Arc::downgrade(&node))
    }

    pub fn new_from_block_devices(
        block_devices: &mut Vec<Box<dyn BlockDevice + Send + Sync>>,
    ) -> Result<Self, FsError> {
        // this stores the found superblock for initialization and index of the block device that contains
        // it for removal
        let mut found = None;

        for (i, block_device) in block_devices.iter_mut().enumerate() {
            const SUPERBLOCK_START: usize = 1024;
            const SUPERBLOCK_SIZE: usize = 1024;
            let mut buf = [0u8; SUPERBLOCK_SIZE];
            if block_device.read(SUPERBLOCK_START, &mut buf).is_err() {
                continue;
            }

            if let Ok((superblock, _)) = Superblock::read_from_prefix(&buf)
                && superblock.magic == 0xEF53
                && superblock.log_block_size <= 2
                && superblock.rev_level == 1
            {
                found = Some((superblock, i));
                break;
            }
        }

        if let Some((superblock, i)) = found {
            return Ok(Self {
                block_size: 1024 << superblock.log_block_size,
                block_device: IntMutex::new(block_devices.swap_remove(i)),
                superblock,
                fnode_cache: IntMutex::new(BTreeMap::new()),
                block_map_lock: IntMutex::new(()),
                inode_map_lock: IntMutex::new(()),
                group_lock: IntMutex::new(()),
            });
        } else {
            return Err(FsError::NotFound);
        }
    }

    fn read_block(self: &Arc<Self>, block_number: usize, buffer: &mut [u8]) -> Result<(), FsError> {
        self.check_block_inputs(block_number, buffer.len())?;

        let mut block_device = self.block_device.lock();

        // read a block into the buffer, returning an error if the read fails or doesn't return a full block. Use read to not have
        // to deal with different block sizes
        if let Ok(bytes_read) = block_device.read(
            block_number * self.block_size,
            &mut buffer[0..self.block_size],
        ) && bytes_read == self.block_size
        {
            return Ok(());
        }
        Err(FsError::ReadError)
    }

    fn write_block(self: &Arc<Self>, block_number: usize, buffer: &[u8]) -> Result<(), FsError> {
        self.check_block_inputs(block_number, buffer.len())?;

        let mut block_device = self.block_device.lock();

        // same as above
        if let Ok(bytes_written) =
            block_device.write(block_number * self.block_size, &buffer[0..self.block_size])
            && bytes_written == self.block_size
        {
            return Ok(());
        }
        Err(FsError::WriteError)
    }

    fn check_block_inputs(
        self: &Arc<Self>,
        block_number: usize,
        buffer_len: usize,
    ) -> Result<(), FsError> {
        if block_number >= self.superblock.blocks_count as usize {
            return Err(FsError::NotFound);
        } else if buffer_len < self.block_size {
            return Err(FsError::InvalidInput);
        }
        Ok(())
    }
}

impl FNode {
    fn block_tree(
        self: &Arc<Self>,
        block_number: usize,
        scratch_buffer: Option<&mut [u8]>,
        inode: &INode,
    ) -> Result<([usize; 4], [usize; 4], usize), FsError> {
        let mut list = [0usize; 4];
        let (indices, depth) = self.block_tree_indices(block_number);
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.fs.block_size])[..],
        };

        list[0] = inode.data.block[indices[0]] as usize;
        for i in 1..depth {
            if list[i - 1] == 0 {
                return Ok((list, indices, depth));
            }
            self.fs.read_block(list[i - 1], scratch_buffer)?;
            let index = indices[i];
            let start = index * 4;
            let end = start + 4;
            let (next_bn, _) = u32::read_from_prefix(&scratch_buffer[start..end]).unwrap();
            list[i] = next_bn as usize;
        }
        Ok((list, indices, depth))
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

    pub fn read_block(
        self: &Arc<Self>,
        block_number: usize,
        buffer: &mut [u8],
        inode: &INode,
    ) -> Result<(), FsError> {
        let (tree, _, size) = self.block_tree(block_number, Some(buffer), inode)?;
        match tree[size - 1] {
            0 => {
                buffer[0..self.fs.block_size].fill(0);
                Ok(())
            }
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
    ) -> Result<(), FsError> {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.fs.block_size])[..],
        };
        let (mut tree, indices, size) =
            self.block_tree(block_number, Some(scratch_buffer), inode)?;
        for i in 0..size {
            if !(tree[i] == 0 || (i == 0 && new_size.is_some())) {
                continue;
            }

            tree[i] = self.fs.alloc_block(preferred_group, Some(scratch_buffer))?;
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
                self.update_inode(inode, new_inode, Some(scratch_buffer))?;
            } else {
                scratch_buffer[0..self.fs.block_size].fill(0);
                (tree[i] as u32)
                    .write_to_prefix(&mut scratch_buffer[indices[i] * 4..])
                    .unwrap();
                self.fs.write_block(tree[i - 1], scratch_buffer)?;
            }
        }
        self.fs.write_block(tree[size - 1], buffer)
    }

    fn update_inode(
        self: &Arc<Self>,
        old: &mut INode,
        new: INode,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<(), FsError> {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.fs.block_size])[..],
        };
        let (_, (inode_block, inode_offset)) =
            self.fs.get_inode(old.number as u32, Some(scratch_buffer))?;
        new.data
            .write_to_prefix(&mut scratch_buffer[inode_offset..])
            .unwrap();
        self.fs.write_block(inode_block, scratch_buffer)?;
        *old = new;
        Ok(())
    }

    // use indexing
    pub fn create_entry(
        self: &Arc<Self>,
        entry_name: &str,
        inumber: u32,
        file_type: u8,
    ) -> Result<(), FsError> {
        if inumber == 0 {
            return Err(FsError::InvalidInput);
        }
        let mut inode = self.inode.lock();
        // TODO proper types
        if inode.data.mode & 0xF000 != 0x4000 {
            return Err(FsError::InvalidInput);
        }
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
                self.read_block(needed_block, &mut buffer, &inode)?;
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
                    return Err(FsError::AlreadyExists);
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
            self.read_block(bn, &mut buffer, &inode)?;
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
        self.write_block(bn, &buffer, &mut inode, None, new_size, ideal_group)?;
        Ok(())
    }

    // TODO: use indexing instead of linsearch
    pub fn search(self: &Arc<Self>, next: &str) -> Result<Weak<FNode>, FsError> {
        let inode = self.inode.lock();
        // TODO proper types
        if inode.data.mode & 0xF000 != 0x4000 {
            return Err(FsError::InvalidInput);
        }
        let mut pointer: usize = 0;
        let mut last_fetched_block: usize = 1;
        let mut buffer = alloc::vec![0u8; self.fs.block_size];
        while pointer < (inode.data.size as usize) {
            assert!(pointer.is_multiple_of(4));
            let needed_block = pointer / self.fs.block_size;
            if needed_block != last_fetched_block {
                self.read_block(needed_block, &mut buffer, &inode)?;
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
                    return self.fs.get_fnode(inumber, Some(&mut buffer[..]));
                }
            }
            pointer += rec_len as usize;
        }
        Err(FsError::NotFound)
    }
}

#[cfg(test)]
mod test {
    use crate::alloc::string::ToString;
    use crate::fs::ext2::Ext2;
    use crate::fs::ramdisk::Ramdisk;
    use crate::print::kprintln;
    use crate::sync::MutexLike;
    use alloc::sync::Arc;

    fn nul_terminated_utf8(buf: &[u8]) -> &str {
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        core::str::from_utf8(&buf[..len]).unwrap()
    }

    #[test_case]
    fn test_block_allocation() {
        let disk = Ramdisk::new(512);
        let fs = Arc::new(Ext2::new(disk).unwrap());
        let root = fs.get_root().unwrap().upgrade().unwrap();
        let hello = root.search("hello").unwrap().upgrade().unwrap();
        let mut buffer = alloc::vec![0u8; fs.block_size];
        {
            let mut inode = hello.inode.lock();
            hello.read_block(0, &mut buffer, &inode).unwrap();
            let original = nul_terminated_utf8(&buffer);
            assert_eq!(original, "world!");
            kprintln!("ext2 hello: {}", original);
            buffer[0] = b'b';
            hello
                .write_block(
                    999,
                    &buffer,
                    &mut inode,
                    None,
                    Some(fs.block_size * 1000),
                    0,
                )
                .unwrap();
            hello.read_block(999, &mut buffer, &inode).unwrap();
            let updated = nul_terminated_utf8(&buffer);
            assert_eq!(updated, "borld!");
            kprintln!("ext2 updated: {}", updated);
        }
    }

    #[test_case]
    fn test_create_entries() {
        let disk = Ramdisk::new(512);
        let fs = Arc::new(Ext2::new(disk).unwrap());
        let root = fs.get_root().unwrap().upgrade().unwrap();
        for i in 0i32..100 {
            root.create_entry(&i.to_string(), 1, 0).unwrap();
        }

        for i in 0i32..100 {
            assert!(root.search(&i.to_string()).is_ok());
        }

        kprintln!("ext2 entries verified: 100");
    }
}
