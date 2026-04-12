extern crate alloc;
use alloc::{
    boxed::Box,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use spin::Once;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use super::vfs::{Filesystem, FsError, INodeKey, INodeType, VNode};
use crate::{
    arch::{Arch, ArchTrait},
    devices::block::BlockDevice,
    memory::virtual_memory::{PagingOptions, VirtualMemoryAllocation},
    sync::{IntMutex, MutexLike},
};

pub struct Ext2 {
    block_size: usize,
    block_device: IntMutex<Box<dyn BlockDevice + Send + Sync>>,
    superblock: Superblock,
    block_map_lock: IntMutex<()>,
    inode_map_lock: IntMutex<()>,
    group_lock: IntMutex<()>,
    vfs_id: IntMutex<Option<usize>>, // To allow for identification in the vfs when it is mounted, perhaps there are better ways to do this
    self_ref: Once<Weak<Self>>, // to allow for getting Arc<Self> in functions that need it, this is set at the end of initialization
}

pub struct FNode {
    // takes an Arc to the filesystem so fnode can call functions on it. Maybe should be a weak?
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
        &self,
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
        &self,
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
        &self,
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
        &self,
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
        &self,
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
        &self,
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

    /// Write an inode's data back to disk.
    pub fn write_inode_data(&self, inode: &INode) -> Result<(), FsError> {
        let scratch_buffer = &mut (alloc::vec![0u8; self.block_size])[..];
        let (_, (inode_block, inode_offset)) =
            self.get_ext2_inode(inode.number as u32, Some(scratch_buffer))?;

        // TODO: handle write failures more cleanly instead of unwrap.
        inode
            .data
            .write_to_prefix(&mut scratch_buffer[inode_offset..])
            .unwrap();
        self.write_block(inode_block, scratch_buffer)?;
        Ok(())
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
        }
        None
    }

    fn get_block_group_descriptor(
        &self,
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

    // from an inumber get an inode, writing its data into the scratch buffer if provided
    fn get_ext2_inode(
        &self,
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

    // for get root and get fnode, we need Arc<Self> so we can give the fnode a reference to the filesystem
    pub fn get_ext2_root(&self) -> Result<Arc<FNode>, FsError>
    where
        Self: Sized,
    {
        self.get_fnode(2, None)
    }

    // get an fnode from an inumber. Note that this used to take Arc<Self> for the filesystem to hand out references to itself, but
    // this made it hard to call from the filesystem trait, so now it just upgrades a weak pointer to itself to hand out copies
    fn get_fnode(
        &self,
        inumber: u32,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<Arc<FNode>, FsError> {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };

        let (inode, _) = self.get_ext2_inode(inumber, Some(scratch_buffer))?;

        // we want each fnode to have an Arc to the filesystem so they can call functions on it, so we upgrade the weak pointer
        // the fs has to itself to give it out
        let node = Arc::new(FNode {
            fs: self
                .self_ref
                .get()
                .ok_or(FsError::NotFound)?
                .upgrade()
                .ok_or(FsError::NotFound)?,
            inode: IntMutex::new(inode),
        });

        Ok(node)
    }

    pub fn new_from_block_devices(
        block_devices: &mut Vec<Box<dyn BlockDevice + Send + Sync>>,
    ) -> Result<Arc<Self>, FsError> {
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
            let ext2 = Arc::new(Self {
                block_size: 1024 << superblock.log_block_size,
                block_device: IntMutex::new(block_devices.swap_remove(i)),
                superblock,
                block_map_lock: IntMutex::new(()),
                inode_map_lock: IntMutex::new(()),
                group_lock: IntMutex::new(()),
                vfs_id: IntMutex::new(None),
                self_ref: Once::new(),
            });
            ext2.self_ref.call_once(|| Arc::downgrade(&ext2));
            Ok(ext2)
        } else {
            Err(FsError::NotFound)
        }
    }

    fn read_block(&self, block_number: usize, buffer: &mut [u8]) -> Result<(), FsError> {
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

    fn write_block(&self, block_number: usize, buffer: &[u8]) -> Result<(), FsError> {
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

    // small helper for read_block and write_block
    fn check_block_inputs(&self, block_number: usize, buffer_len: usize) -> Result<(), FsError> {
        if block_number >= self.superblock.blocks_count as usize {
            return Err(FsError::NotFound);
        } else if buffer_len < self.block_size {
            return Err(FsError::InvalidInput);
        }
        Ok(())
    }
}

impl Filesystem for Ext2 {
    fn get_root(&self) -> Result<Arc<dyn VNode>, FsError> {
        let root = self.get_ext2_root()?;
        Ok(root)
    }

    fn get_inode(&self, inumber: usize) -> Result<Arc<dyn VNode>, FsError> {
        let inode = self.get_fnode(inumber as u32, None)?;
        Ok(inode)
    }

    fn get_filesystem_id(&self) -> Result<usize, FsError> {
        self.vfs_id.lock().ok_or(FsError::NotFound)
    }

    fn set_filesystem_id(&self, id: Option<usize>) {
        let mut vfs_id = self.vfs_id.lock();
        *vfs_id = id;
    }
}

impl FNode {
    fn block_tree(
        &self,
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

    fn block_tree_indices(&self, block_number: usize) -> ([usize; 4], usize) {
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
        &self,
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
        &self,
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
        &self,
        old: &mut INode,
        new: INode,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Result<(), FsError> {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.fs.block_size])[..],
        };
        let (_, (inode_block, inode_offset)) = self
            .fs
            .get_ext2_inode(old.number as u32, Some(scratch_buffer))?;
        new.data
            .write_to_prefix(&mut scratch_buffer[inode_offset..])
            .unwrap();
        self.fs.write_block(inode_block, scratch_buffer)?;
        *old = new;
        Ok(())
    }

    // use indexing
    pub fn create_entry(
        &self,
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
    pub fn search(&self, file_name: &str) -> Result<Arc<FNode>, FsError> {
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
                if name == file_name {
                    return self.fs.get_fnode(inumber, Some(&mut buffer[..]));
                }
            }
            pointer += rec_len as usize;
        }
        Err(FsError::NotFound)
    }
}

impl VNode for FNode {
    // Files
    fn get_inumber(&self) -> usize {
        self.inode.lock().number
    }

    fn get_type(&self) -> INodeType {
        let inode = self.inode.lock();
        match inode.data.mode & 0xF000 {
            0x4000 => INodeType::Directory,
            0x8000 => INodeType::File,
            _ => INodeType::Other,
        }
    }

    // create a child and return the VNode, this is used for both creating files and directories
    fn create_child(&self, name: &str, inode_type: INodeType) -> Result<Arc<dyn VNode>, FsError> {
        if self.get_type() != INodeType::Directory {
            return Err(FsError::InvalidInput);
        }
        let file_type = match inode_type {
            INodeType::Directory => 2,
            INodeType::File => 1,
            _ => return Err(FsError::InvalidInput),
        } as u8;
        let inumber = self.fs.alloc_inode(
            (self.inode.lock().number - 1) / (self.fs.superblock.inodes_per_group as usize),
            None,
        )? as u32;

        // TODO not handling these fields correctly currently, minimum working impl
        // TODO I believe the parent directory's link count should increase as well..?
        let inode = INode {
            number: inumber as usize,
            data: INodeData {
                mode: match inode_type {
                    INodeType::Directory => 0x4000,
                    INodeType::File => 0x8000,
                    _ => unreachable!(),
                },
                uid: 0,
                size: 0,
                atime: 0,
                ctime: 0,
                mtime: 0,
                dtime: 0,
                gid: 0,
                links_count: if inode_type == INodeType::Directory {
                    2
                } else {
                    1
                },
                blocks: 0,
                flags: 0,
                osd1: 0,
                block: [0; 15],
                generation: 0,
                file_acl: 0,
                dir_acl: 0,
                faddr: 0,
                osd2: [0; 12],
            },
            dirty: false,
        };

        if let Err(fs_error) = self.fs.write_inode_data(&inode) {
            // if we fail to write the inode data, we need to deallocate the inode we just allocated to avoid leaks
            let _ = self.fs.dealloc_inode(inumber as usize, None);
            return Err(fs_error);
        }
        if let Err(fs_error) = self.create_entry(name, inumber, file_type) {
            // if we fail to create the directory entry, we also need to deallocate the inode we just allocated
            let _ = self.fs.dealloc_inode(inumber as usize, None);
            return Err(fs_error);
        }
        let fnode = FNode {
            fs: self.fs.clone(),
            inode: IntMutex::new(inode),
        };

        if fnode.get_type() == INodeType::Directory {
            // if it's a directory, we need to create the "." and ".." entries
            if let Err(fs_error) = fnode.create_entry(".", inumber, file_type) {
                // if we fail to create the "." entry, we need to clean up both the inode and the directory entry we just created
                // TODO implement directory entry deletion to avoid dangling directory entry
                let _ = self.fs.dealloc_inode(inumber as usize, None);
                return Err(fs_error);
            }
            if let Err(fs_error) =
                fnode.create_entry("..", self.inode.lock().number as u32, file_type)
            {
                // if we fail to create the ".." entry, we need to clean up the inode and the entries we just created
                let _ = self.fs.dealloc_inode(inumber as usize, None);
                return Err(fs_error);
            }
        }

        Ok(Arc::new(fnode))
    }

    // TODO implement some kind of check to make sure the physical address is valid
    // this if unfortunately still a pretty ugly function.
    fn read_page(&self, physical_address: usize, offset: usize) -> Result<usize, FsError> {
        let options = PagingOptions::PRESENT | PagingOptions::WRITABLE;
        let allocation = VirtualMemoryAllocation::new(
            Arch::get_kernel_address_space(),
            None,
            Arch::PAGE_SIZE,
            Some(physical_address),
            options,
            true,
        )
        .ok_or(FsError::Other(String::from("vm allocation failed")))?;
        let virt_addr = allocation.base;

        // Safety: we trust our virtual memory allocator and this won't be reused until after allocation is freed
        let page_buf =
            unsafe { core::slice::from_raw_parts_mut(virt_addr as *mut u8, Arch::PAGE_SIZE) };

        let inode = self.inode.lock();
        let file_size = inode.data.size as usize;
        let block_size = self.fs.block_size;

        // Zero-fill the whole page up front so anything past EOF or in sparse
        // regions is already correct.
        page_buf.fill(0);

        if offset >= file_size {
            return Ok(0);
        }

        let total_to_read = core::cmp::min(Arch::PAGE_SIZE, file_size - offset);
        let mut read_so_far = 0;

        // Scratch buffer only for edge partial-block reads.
        let mut scratch = alloc::vec![0u8; block_size];

        while read_so_far < total_to_read {
            let file_pos = offset + read_so_far;
            let block_number = file_pos / block_size;
            let block_offset = file_pos % block_size;

            // we can read at most the rest of the file, or the rest of the block, whichever is smaller.
            let chunk_len = core::cmp::min(total_to_read - read_so_far, block_size - block_offset);
            if block_offset == 0 && chunk_len == block_size {
                // if the whole block is needed and we're block-aligned, we can read directly into the page buffer
                self.read_block(
                    block_number,
                    &mut page_buf[read_so_far..read_so_far + block_size],
                    &inode,
                )?;
            } else {
                // any other case, we need to read into a scratch buffer and then copy the relevant portion to the page buffer
                self.read_block(block_number, &mut scratch, &inode)?;
                page_buf[read_so_far..read_so_far + chunk_len]
                    .copy_from_slice(&scratch[block_offset..block_offset + chunk_len]);
            }

            read_so_far += chunk_len;
        }

        Ok(read_so_far)
    }

    // write page across multiple blocks. We want this to be done all at once over multiple blocks, but fs still doesn't have
    // a clean way to do this
    fn write_page(&self, physical_address: usize, offset: usize) -> Result<usize, FsError> {
        let options = PagingOptions::PRESENT | PagingOptions::WRITABLE;
        let allocation = VirtualMemoryAllocation::new(
            Arch::get_kernel_address_space(),
            None,
            Arch::PAGE_SIZE,
            Some(physical_address),
            options,
            true,
        )
        .ok_or(FsError::Other(String::from("vm allocation failed")))?;
        let virt_addr = allocation.base;

        // Safety: this mapping will stay alive for the duration of
        // the function, and it is exactly one page long.
        let page = unsafe { core::slice::from_raw_parts(virt_addr as *const u8, Arch::PAGE_SIZE) };

        let mut inode = self.inode.lock();
        let block_size = self.fs.block_size;

        // place new blocks in the group of the file's inode
        let preferred_group = (inode.number - 1) / (self.fs.superblock.inodes_per_group as usize);

        let mut written = 0;
        let mut scratch = alloc::vec![0u8; block_size];

        // different from read_page, we can write past EOF
        while written < Arch::PAGE_SIZE {
            let file_pos = offset + written;
            let block_number = file_pos / block_size;
            let block_offset = file_pos % block_size;
            let chunk_len = core::cmp::min(Arch::PAGE_SIZE - written, block_size - block_offset);

            if block_offset == 0 && chunk_len == block_size {
                // fast path: whole filesystem block can be written directly.
                self.write_block(
                    block_number,
                    &page[written..written + block_size],
                    &mut inode,
                    None,
                    None,
                    preferred_group,
                )?;
            } else {
                // partial block: preserve bytes outside the written subrange. read modify write cycle
                self.read_block(block_number, &mut scratch, &inode)?;

                scratch[block_offset..block_offset + chunk_len]
                    .copy_from_slice(&page[written..written + chunk_len]);

                self.write_block(
                    block_number,
                    &scratch,
                    &mut inode,
                    None,
                    None,
                    preferred_group,
                )?;
            }

            written += chunk_len;
        }

        // update the inode size
        let new_size = core::cmp::max(inode.data.size as usize, offset + Arch::PAGE_SIZE);
        if new_size != inode.data.size as usize {
            let new_inode = INode {
                number: inode.number,
                dirty: true,
                data: INodeData {
                    size: new_size as u32,
                    ..inode.data
                },
            };
            self.update_inode(&mut inode, new_inode, None)?;
        }

        Ok(written)
    }

    fn size(&self) -> usize {
        let inode = self.inode.lock();
        inode.data.size as usize
    }

    // Directory
    fn lookup(&self, target: &str) -> Result<Arc<dyn VNode>, FsError> {
        if self.get_type() != INodeType::Directory {
            return Err(FsError::InvalidOperation);
        }
        let fnode = self.search(target)?;
        Ok(fnode)
    }

    fn add_entry(
        &self,
        target: &str,
        inumber: usize,
        inode_type: INodeType,
    ) -> Result<(), FsError> {
        if self.get_type() != INodeType::Directory {
            return Err(FsError::InvalidOperation);
        }
        let file_type = match inode_type {
            INodeType::Directory => 2,
            INodeType::File => 1,
            INodeType::Other => 0,
        };
        self.create_entry(target, inumber as u32, file_type)
    }

    // read across blocks starting at offset.
    fn read_unaligned(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, FsError> {
        let mut total_read = 0;
        let mut scratch = alloc::vec![0u8; self.fs.block_size];
        let inode = self.inode.lock();
        let file_size = inode.data.size as usize;
        if offset >= file_size {
            return Ok(0);
        }
        while total_read < buffer.len() && offset + total_read < file_size {
            let file_pos = offset + total_read;
            let block_number = file_pos / self.fs.block_size;
            let block_offset = file_pos % self.fs.block_size;
            self.read_block(block_number, &mut scratch, &inode)?;

            // copy at most the rest of the block, the caller buffer, and remaining file.
            let to_copy = core::cmp::min(
                core::cmp::min(buffer.len() - total_read, self.fs.block_size - block_offset),
                file_size - file_pos,
            );
            buffer[total_read..total_read + to_copy]
                .copy_from_slice(&scratch[block_offset..block_offset + to_copy]);
            total_read += to_copy;
        }
        Ok(total_read)
    }

    // write across blocks starting at offset.
    fn write_unaligned(&self, offset: usize, buffer: &[u8]) -> Result<usize, FsError> {
        let mut total_written = 0;
        let mut scratch = alloc::vec![0u8; self.fs.block_size];
        let mut inode = self.inode.lock();

        while total_written < buffer.len() {
            let file_pos = offset + total_written;
            let block_number = file_pos / self.fs.block_size;
            let block_offset = file_pos % self.fs.block_size;
            self.read_block(block_number, &mut scratch, &inode)?;
            let to_copy = core::cmp::min(
                buffer.len() - total_written,
                self.fs.block_size - block_offset,
            );
            scratch[block_offset..block_offset + to_copy]
                .copy_from_slice(&buffer[total_written..total_written + to_copy]);
            self.write_block(block_number, &scratch, &mut inode, None, None, 0)?;
            total_written += to_copy;
        }

        let new_size = core::cmp::max(inode.data.size as usize, offset + buffer.len());
        if new_size != inode.data.size as usize {
            let new_inode = INode {
                number: inode.number,
                dirty: true,
                data: INodeData {
                    size: new_size as u32,
                    ..inode.data
                },
            };
            self.update_inode(&mut inode, new_inode, None)?;
        }

        Ok(total_written)
    }

    fn get_inode_key(&self) -> Result<INodeKey, FsError> {
        Ok(INodeKey {
            filesystem_id: self.fs.get_filesystem_id()?,
            inumber: self.inode.lock().number,
        })
    }
}
