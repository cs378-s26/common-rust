extern crate alloc;

use limine::request::ModuleRequest;
use zerocopy::{FromBytes, Immutable, KnownLayout};

#[used]
#[unsafe(link_section = ".limine_requests")]
static MODULE_REQUEST: ModuleRequest = ModuleRequest::new();

const PAGE_SIZE: usize = 4096;

trait Disk {
    fn read_sector(self: &Self, sector: usize, buffer: &mut [u8]);
    fn write_sector(self: &mut Self, sector: usize, buffer: &[u8]);
    fn sector_size(self: &Self) -> usize;
}

struct GoofyDisk<'a> {
    base_address: &'a mut [u8],
    sector_size: usize,
}

impl GoofyDisk<'static> {
    fn new(sector_size: usize) -> Self {
        let response = MODULE_REQUEST
            .get_response()
            .expect("could not load modules (needed for fs)");
        let module = response.modules()[1];
        let addr = module.addr();
        let size = module.size() as usize;
        assert!(addr.align_offset(sector_size) == 0);
        assert!(size % sector_size == 0);
        assert!(PAGE_SIZE % sector_size == 0);
        unsafe {
            return Self {
                base_address: core::slice::from_raw_parts_mut(addr, size),
                sector_size: sector_size,
            };
        }
    }
}

impl Disk for GoofyDisk<'static> {
    fn read_sector(self: &Self, sector: usize, buffer: &mut [u8]) {
        assert!(sector < self.base_address.len() / self.sector_size);
        let start = sector * self.sector_size;
        let end = start + self.sector_size;
        buffer[..self.sector_size].copy_from_slice(&self.base_address[start..end]);
    }

    fn sector_size(self: &Self) -> usize {
        self.sector_size
    }

    fn write_sector(self: &mut Self, sector: usize, buffer: &[u8]) {
        assert!(sector < self.base_address.len() / self.sector_size);
        let start = sector * self.sector_size;
        let end = start + self.sector_size;
        self.base_address[start..end].copy_from_slice(&buffer[..self.sector_size]);
    }
}

#[repr(C, packed)]
#[derive(Clone, FromBytes, KnownLayout, Immutable)]
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

struct Ext2Node<'a, D: Disk> {
    fs: &'a Ext2<D>,
    data: INode,
}

impl<'a, D: Disk> Ext2Node<'a, D> {
    fn read_block(self: &Self, block_number: usize, buffer: &mut [u8]) {
        assert!(buffer.len() >= self.fs.block_size);
        match self.translate_block_number(block_number, Some(buffer)) {
            Some(bn) => {
                self.fs.read_block(bn, buffer);
            }
            None => {
                (&mut buffer[0..self.fs.block_size]).fill(0);
            }
        }
    }

    fn resolve_indirect_block(
        self: &Self,
        indirect_block: usize,
        index: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> usize {
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0 as u8; self.fs.block_size])[..],
        };
        self.fs.read_block(indirect_block, scratch_buffer);
        let start = index * 4;
        let end = start + 4;
        let (result, _) = u32::read_from_prefix(&scratch_buffer[start..end]).unwrap();
        result as usize
    }

    fn translate_block_number(
        self: &Self,
        from_block: usize,
        scratch_buffer: Option<&mut [u8]>,
    ) -> Option<usize> {
        assert!(from_block < (self.data.blocks as usize));

        // no indirection
        if from_block < 12 {
            let to_block = self.data.block[from_block] as usize;
            return if to_block != 0 { Some(to_block) } else { None };
        }

        // setting up
        let scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0 as u8; self.fs.block_size])[..],
        };
        let x = self.fs.block_size / 4;

        // one level indirection
        let from_block = from_block - 12;
        if from_block < x {
            let to_block = self.data.block[12] as usize;
            if to_block == 0 {
                return None;
            }
            let to_block = self.resolve_indirect_block(to_block, from_block, Some(scratch_buffer));
            return if to_block != 0 { Some(to_block) } else { None };
        }

        // two level indirection
        let from_block = from_block - x;
        if from_block < x * x {
            let to_block = self.data.block[13] as usize;
            if to_block == 0 {
                return None;
            }
            let to_block =
                self.resolve_indirect_block(to_block, from_block / x, Some(scratch_buffer));
            if to_block == 0 {
                return None;
            }
            let to_block =
                self.resolve_indirect_block(to_block, from_block % x, Some(scratch_buffer));
            return if to_block != 0 { Some(to_block) } else { None };
        }

        // three level indirection
        let from_block = from_block - x * x;
        if from_block < x * x * x {
            let to_block = self.data.block[14] as usize;
            if to_block == 0 {
                return None;
            }
            let to_block =
                self.resolve_indirect_block(to_block, from_block / (x * x), Some(scratch_buffer));
            if to_block == 0 {
                return None;
            }
            let to_block =
                self.resolve_indirect_block(to_block, from_block / x % x, Some(scratch_buffer));
            if to_block == 0 {
                return None;
            }
            let to_block =
                self.resolve_indirect_block(to_block, from_block % (x * x), Some(scratch_buffer));
            return if to_block != 0 { Some(to_block) } else { None };
        }
        return None;
    }

    fn traverse(self: &Self, next: &str) -> Option<Ext2Node<'a, D>> {
        assert!(self.data.mode & 0xF000 == 0x4000);
        let mut pointer: usize = 0;
        let mut last_fetched_block: usize = 1;
        let mut buffer = alloc::vec![0 as u8; self.fs.block_size];
        while pointer < (self.data.size as usize) {
            assert!(pointer % 4 == 0);
            let needed_block = pointer / self.fs.block_size;
            if needed_block != last_fetched_block {
                self.read_block(needed_block, &mut buffer);
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
                        return Some(Ext2Node {
                            fs: self.fs,
                            data: self.fs.get_inode(inumber, Some(&mut buffer)),
                        });
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

#[repr(C, packed)]
#[derive(FromBytes, KnownLayout, Immutable)]
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

struct Ext2<D: Disk> {
    block_size: usize,
    disk: D,
    superblock: Superblock,
}

#[repr(C, packed)]
#[derive(FromBytes, KnownLayout, Immutable)]
struct BlockGroupDescriptor {
    block_bitmap: u32,
    inode_bitmap: u32,
    inode_table: u32,
    free_blocks_count: u16,
    free_inodes_count: u16,
    used_dirs_count: u16,
}

impl<D: Disk> Ext2<D> {
    fn get_block_group_descriptor(
        self: &Self,
        block_group: u32,
        scratch_buffer: Option<&mut [u8]>,
    ) -> BlockGroupDescriptor {
        let block_group = block_group as usize;
        const BGD_SIZE: usize = 4;
        let descriptors_per_block = self.block_size / BGD_SIZE;
        let bgd_block =
            (self.superblock.first_data_block as usize) + 1 + block_group / descriptors_per_block;
        let mut scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0 as u8; self.block_size])[..],
        };
        self.read_block(bgd_block, &mut scratch_buffer);
        let bgd_index = block_group % descriptors_per_block;
        let bgd_offset = bgd_index * BGD_SIZE;
        let (bgd, _) =
            BlockGroupDescriptor::read_from_prefix(&scratch_buffer[bgd_offset..]).unwrap();
        bgd
    }

    fn get_inode(self: &Self, inumber: u32, scratch_buffer: Option<&mut [u8]>) -> INode
    where
        Self: Sized,
    {
        assert!(inumber > 0);
        let mut scratch_buffer = match scratch_buffer {
            Some(s) => s,
            None => &mut (alloc::vec![0 as u8; self.block_size])[..],
        };
        let block_group = (inumber - 1) / self.superblock.inodes_per_group;
        let bgd = self.get_block_group_descriptor(block_group, Some(scratch_buffer));
        let inodes_per_block = self.block_size / (self.superblock.inode_size as usize);
        let inode_index = ((inumber - 1) % self.superblock.inodes_per_group) as usize;
        let inode_block = (bgd.inode_table as usize) + inode_index / inodes_per_block;
        self.read_block(inode_block, &mut scratch_buffer);
        let inode_index = inode_index % inodes_per_block;
        let inode_offset = inode_index * (self.superblock.inode_size as usize);
        let (inode, _) = INode::read_from_prefix(&scratch_buffer[inode_offset..]).unwrap();
        inode
    }

    fn get_root(self: &Self) -> Ext2Node<'_, D>
    where
        Self: Sized,
    {
        let data = self.get_inode(2, None);
        Ext2Node {
            fs: self,
            data: data,
        }
    }

    fn new(disk: D) -> Result<Self, &'static str> {
        if disk.sector_size() < 512 {
            return Err("sector size not big enough");
        }

        // get the superblock
        const SUPERBLOCK_START: usize = 1024;
        let superblock_sector = SUPERBLOCK_START / disk.sector_size();
        let superblock_offset = SUPERBLOCK_START % disk.sector_size();
        let mut buffer = alloc::vec![0 as u8; disk.sector_size()];
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
            disk: disk,
            superblock: superblock,
        })
    }

    fn read_block(self: &Self, block_number: usize, buffer: &mut [u8]) {
        assert!(buffer.len() >= self.block_size);
        let sector_size = self.disk.sector_size();
        let factor = self.block_size / sector_size;
        for i in 0..factor {
            let start = i * sector_size;
            let end = start + sector_size;
            self.disk
                .read_sector(block_number * factor + i, &mut buffer[start..end]);
        }
    }
}

pub fn fs_init() {
    let disk = GoofyDisk::new(512);
    let fs = Ext2::new(disk);
    let fs = fs.unwrap();
    let root = fs.get_root();
    crate::print::kprintln!("===START===");
    let file = root.traverse("hello");
    match file {
        Some(f) => {
            crate::print::kprintln!("found!");
            crate::print::kprintln!("contents of first block:");
            let mut buffer = alloc::vec![0 as u8; f.fs.block_size];
            f.read_block(0, &mut buffer);
            let content = core::str::from_utf8(&buffer[0..f.data.size as usize]);
            crate::print::kprintln!("{}", content.unwrap());
        }
        None => crate::print::kprintln!("not found."),
    };
    crate::print::kprintln!("=== END ===");
}
