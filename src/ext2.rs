extern crate alloc;

use crate::ramdisk::Disk;
use alloc::{collections::btree_map::BTreeMap, sync::Arc, sync::Weak};
use spin::Mutex;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

pub struct Ext2<D: Disk> {
    block_size: usize,
    disk: Mutex<D>,
    superblock: Superblock,
    fnode_cache: Mutex<BTreeMap<u32, Arc<FNode<D>>>>,
    block_map_lock: Mutex<()>,
    group_lock: Mutex<()>,
}

pub struct FNode<D: Disk> {
    fs: Arc<Ext2<D>>,
    inode: Mutex<INode>,
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

#[repr(C, packed)]
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable)]
struct INode {
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

impl<D: Disk> Ext2<D> {
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

    fn dealloc_block(self: &Arc<Self>, block_number: usize, scratch_buffer: Option<&mut [u8]>) {
        let _guard = self.block_map_lock.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        let block_number = block_number - (self.superblock.first_data_block as usize);
        let group = block_number / (self.superblock.blocks_per_group as usize);
        let (bgd, (bgd_offset, bgd_block)) =
            self.get_block_group_descriptor(group, Some(scratch_buffer));
        let bitmap = {
            let _guard = self.group_lock.lock();
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
        let block_group = block_group as usize;
        const BGD_SIZE: usize = 4;
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

    fn get_inode(self: &Arc<Self>, inumber: u32, scratch_buffer: Option<&mut [u8]>) -> INode
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
        let (inode, _) = INode::read_from_prefix(&scratch_buffer[inode_offset..]).unwrap();
        inode
    }

    fn get_root(self: &Arc<Self>) -> Weak<FNode<D>>
    where
        Self: Sized,
    {
        self.get_fnode(2, None)
    }

    fn get_fnode(
        self: &Arc<Self>,
        inumber: u32,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Weak<FNode<D>> {
        let mut fnode_cache = self.fnode_cache.lock();
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0u8; self.block_size])[..],
        };
        if let Some(s) = fnode_cache.get(&inumber) {
            return Arc::downgrade(s);
        }
        let inode = self.get_inode(inumber, Some(scratch_buffer));
        let node = Arc::new(FNode {
            fs: self.clone(),
            inode: Mutex::new(inode),
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
            disk: Mutex::new(disk),
            superblock: superblock,
            fnode_cache: Mutex::new(BTreeMap::new()),
            block_map_lock: Mutex::new(()),
            group_lock: Mutex::new(()),
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

impl<D: Disk> FNode<D> {
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

        list[0] = inode.block[indices[0]] as usize;
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
        return (
            [
                14,
                block_number / (bpb * bpb),
                block_number / bpb % bpb,
                block_number % (bpb * bpb),
            ],
            4,
        );
    }

    fn read_block(self: &Arc<Self>, block_number: usize, buffer: &mut [u8], inode: &INode) {
        let (tree, _, size) = self.block_tree(block_number, Some(buffer), &inode);
        match tree[size - 1] {
            0 => (&mut buffer[0..self.fs.block_size]).fill(0),
            b => self.fs.read_block(b, buffer),
        }
    }

    // TODO: use indexing instead of linsearch
    fn traverse(self: &Arc<Self>, next: &str) -> Option<Weak<FNode<D>>> {
        let inode = self.inode.lock();
        assert!(inode.mode & 0xF000 == 0x4000);
        let mut pointer: usize = 0;
        let mut last_fetched_block: usize = 1;
        let mut buffer = alloc::vec![0u8; self.fs.block_size];
        while pointer < (inode.size as usize) {
            assert!(pointer % 4 == 0);
            let needed_block = pointer / self.fs.block_size;
            if needed_block != last_fetched_block {
                self.read_block(needed_block, &mut buffer, &inode);
                last_fetched_block = needed_block;
            }
            let offset = pointer % self.fs.block_size;
            let (inumber, _) = u32::read_from_prefix(&buffer[offset..]).unwrap();
            let (rec_len, _) = u16::read_from_prefix(&buffer[offset + 4..]).unwrap();
            let (name_len, _) = u8::read_from_prefix(&buffer[offset + 6..]).unwrap();
            let name = &buffer[offset + 8..offset + 8 + (name_len as usize)];
            let name = core::str::from_utf8(name);
            match name {
                Ok(s) => {
                    if s == next {
                        return Some(self.fs.get_fnode(inumber, Some(&mut buffer[..])));
                    }
                }
                Err(_) => {
                    panic!("could not parse a string inside a ext2 directory?");
                }
            };
            pointer += rec_len as usize;
        }
        None
    }
}

#[cfg(test)]
mod test {
    use crate::ext2::Ext2;
    use crate::print::kprintln;
    use crate::ramdisk::Ramdisk;
    use alloc::sync::Arc;

    #[test_case]
    fn test_block_allocation() {
        let disk = Ramdisk::new(512);
        let fs = Arc::new(Ext2::new(disk).unwrap());
        let root = fs.get_root().upgrade().unwrap();
        let hello = root.traverse("hello").unwrap().upgrade().unwrap();
        let mut buffer = alloc::vec![0u8; fs.block_size];
        {
            let inode = hello.inode.lock();
            hello.read_block(0, &mut buffer, &inode);
            kprintln!("{}", core::str::from_utf8(&buffer[..]).unwrap());
        }
        kprintln!("{}", fs.alloc_block(0, None).unwrap());
    }
}
