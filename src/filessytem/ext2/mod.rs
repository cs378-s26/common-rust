extern crate alloc;

use alloc::vec;
use core::cmp::min;

use crate::devices::block::BlockDevice;

const EXT2_MAGIC: u16 = 0xEF53;
const ROOT_INODE_NUMBER: u32 = 2;
const INODE_DIRECT_BLOCKS: usize = 12;
const BLOCK_GROUP_DESCRIPTOR_SIZE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ext2Error {
    BlockDevice,
    InvalidSuperblockMagic(u16),
    UnsupportedBlockSize(u32),
    InvalidBufferSize,
    InvalidInodeNumber(u32),
    InvalidPath,
    NotFound,
    NotDirectory(u32),
    NotRegularFile(u32),
    UnsupportedFeature(&'static str),
    Corrupt(&'static str),
    IntegerOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodeKind {
    RegularFile,
    Directory,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct INode {
    inode_number: u32,
    mode: u16,
    size: u32,
    block_pointers: [u32; 15],
}

impl INode {
    pub fn inode_number(&self) -> u32 {
        self.inode_number
    }

    pub fn size(&self) -> usize {
        self.size as usize
    }

    pub fn kind(&self) -> InodeKind {
        match self.mode & 0xF000 {
            0x4000 => InodeKind::Directory,
            0x8000 => InodeKind::RegularFile,
            _ => InodeKind::Other,
        }
    }

    pub fn is_directory(&self) -> bool {
        self.kind() == InodeKind::Directory
    }

    pub fn is_regular_file(&self) -> bool {
        self.kind() == InodeKind::RegularFile
    }

    pub fn read<D: BlockDevice>(
        &self,
        fs: &Ext2<D>,
        file_offset: usize,
        buffer: &mut [u8],
    ) -> Result<usize, Ext2Error> {
        if !self.is_regular_file() && !self.is_directory() {
            return Err(Ext2Error::NotRegularFile(self.inode_number));
        }
        let file_size = self.size as usize;
        if buffer.is_empty() {
            return Ok(0);
        }
        if file_offset >= file_size {
            return Ok(0);
        }

        let to_read = min(buffer.len(), file_size - file_offset);
        let mut bytes_read = 0_usize;
        let mut block_buffer = vec![0_u8; fs.block_size()];

        while bytes_read < to_read {
            let absolute_offset = file_offset
                .checked_add(bytes_read)
                .ok_or(Ext2Error::IntegerOverflow)?;
            let logical_block_index = absolute_offset / fs.block_size();
            let offset_in_block = absolute_offset % fs.block_size();
            let copy_len = min(to_read - bytes_read, fs.block_size() - offset_in_block);

            let fs_block = self.resolve_fs_block_for_file_block(fs, logical_block_index)?;
            if fs_block == 0 {
                buffer[bytes_read..bytes_read + copy_len].fill(0);
            } else {
                fs.read(fs_block, &mut block_buffer)?;
                buffer[bytes_read..bytes_read + copy_len]
                    .copy_from_slice(&block_buffer[offset_in_block..offset_in_block + copy_len]);
            }

            bytes_read += copy_len;
        }

        Ok(bytes_read)
    }

    pub fn find<D: BlockDevice>(&self, fs: &Ext2<D>, path: &str) -> Result<INode, Ext2Error> {
        if path.starts_with('/') {
            return Err(Ext2Error::InvalidPath);
        }

        let mut current = *self;
        if path.is_empty() || path == "." {
            return Ok(current);
        }

        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            if component.len() > 255 {
                return Err(Ext2Error::InvalidPath);
            }
            current = current.lookup_directory_entry(fs, component)?;
        }

        Ok(current)
    }

    fn resolve_fs_block_for_file_block<D: BlockDevice>(
        &self,
        fs: &Ext2<D>,
        logical_block_index: usize,
    ) -> Result<u32, Ext2Error> {
        if logical_block_index < INODE_DIRECT_BLOCKS {
            return Ok(self.block_pointers[logical_block_index]);
        }

        let pointers_per_block = fs.block_size() / core::mem::size_of::<u32>();
        if pointers_per_block == 0 {
            return Err(Ext2Error::Corrupt("invalid ext2 block size"));
        }

        let indirect_index = logical_block_index
            .checked_sub(INODE_DIRECT_BLOCKS)
            .ok_or(Ext2Error::IntegerOverflow)?;
        if indirect_index < pointers_per_block {
            let indirect_block = self.block_pointers[INODE_DIRECT_BLOCKS];
            if indirect_block == 0 {
                return Ok(0);
            }

            let mut indirect = vec![0_u8; fs.block_size()];
            fs.read(indirect_block, &mut indirect)?;

            let entry_offset = indirect_index
                .checked_mul(core::mem::size_of::<u32>())
                .ok_or(Ext2Error::IntegerOverflow)?;
            return read_le_u32(&indirect, entry_offset);
        }

        Err(Ext2Error::UnsupportedFeature(
            "double/triple indirect blocks are not implemented yet",
        ))
    }

    fn lookup_directory_entry<D: BlockDevice>(
        &self,
        fs: &Ext2<D>,
        name: &str,
    ) -> Result<INode, Ext2Error> {
        if !self.is_directory() {
            return Err(Ext2Error::NotDirectory(self.inode_number));
        }

        let mut directory_offset = 0_usize;
        let mut block_buffer = vec![0_u8; fs.block_size()];

        while directory_offset < self.size as usize {
            let valid_bytes = self.read(fs, directory_offset, &mut block_buffer)?;
            if valid_bytes == 0 {
                return Err(Ext2Error::Corrupt(
                    "directory entry stream ended unexpectedly",
                ));
            }
            let mut entry_offset = 0_usize;

            while entry_offset < valid_bytes {
                let inode_number = read_le_u32(&block_buffer, entry_offset)?;
                let rec_len = read_le_u16(&block_buffer, entry_offset + 4)? as usize;
                let name_len = read_u8(&block_buffer, entry_offset + 6)? as usize;

                if rec_len < 8 || rec_len.is_multiple_of(4) {
                    return Err(Ext2Error::Corrupt(
                        "directory entry has an invalid record length",
                    ));
                }
                if entry_offset + rec_len > valid_bytes {
                    return Err(Ext2Error::Corrupt(
                        "directory entry extends past block boundary",
                    ));
                }
                if name_len > rec_len - 8 {
                    return Err(Ext2Error::Corrupt(
                        "directory entry name exceeds record length",
                    ));
                }

                if inode_number != 0 {
                    let name_start = entry_offset + 8;
                    let name_end = name_start + name_len;
                    if &block_buffer[name_start..name_end] == name.as_bytes() {
                        return fs.read_inode(inode_number);
                    }
                }

                entry_offset += rec_len;
            }

            directory_offset = directory_offset
                .checked_add(valid_bytes)
                .ok_or(Ext2Error::IntegerOverflow)?;
        }

        Err(Ext2Error::NotFound)
    }
}

pub struct Ext2<D: BlockDevice> {
    disk: D,
    device_block_size: usize,
    block_size: usize,
    inodes_per_group: u32,
    inode_size: usize,
    bgdt_offset: usize,
    root_inode: INode,
}

impl<D: BlockDevice> Ext2<D> {
    pub fn mount(disk: D) -> Result<Self, Ext2Error> {
        let device_block_size = disk.block_size();
        if device_block_size == 0 {
            return Err(Ext2Error::UnsupportedFeature(
                "device block size must be > 0",
            ));
        }

        let mut superblock = [0_u8; 1024];
        let mut superblock_copied = 0_usize;
        let mut device_block = vec![0_u8; device_block_size];
        while superblock_copied < superblock.len() {
            let absolute_offset = 1024_usize
                .checked_add(superblock_copied)
                .ok_or(Ext2Error::IntegerOverflow)?;
            let disk_block = absolute_offset / device_block_size;
            let offset_in_block = absolute_offset % device_block_size;
            let copy_len = min(
                superblock.len() - superblock_copied,
                device_block_size - offset_in_block,
            );

            let mut buffers: [&mut [u8]; 1] = [&mut device_block];
            disk.read_blocks(&[disk_block], &mut buffers)
                .map_err(|_| Ext2Error::BlockDevice)?;

            superblock[superblock_copied..superblock_copied + copy_len]
                .copy_from_slice(&device_block[offset_in_block..offset_in_block + copy_len]);
            superblock_copied += copy_len;
        }

        let magic = read_le_u16(&superblock, 56)?;
        if magic != EXT2_MAGIC {
            return Err(Ext2Error::InvalidSuperblockMagic(magic));
        }

        let log_block_size = read_le_u32(&superblock, 24)?;
        if log_block_size > 2 {
            return Err(Ext2Error::UnsupportedBlockSize(log_block_size));
        }

        let block_size = 1024_usize
            .checked_shl(log_block_size)
            .ok_or(Ext2Error::IntegerOverflow)?;
        if block_size < 1024 {
            return Err(Ext2Error::Corrupt("ext2 block size smaller than 1024"));
        }

        let inodes_per_group = read_le_u32(&superblock, 40)?;
        if inodes_per_group == 0 {
            return Err(Ext2Error::Corrupt("superblock inodes_per_group is zero"));
        }

        let revision_level = read_le_u32(&superblock, 76)?;
        let inode_size = if revision_level >= 1 {
            let size = read_le_u16(&superblock, 88)? as usize;
            if size < 128 {
                return Err(Ext2Error::Corrupt("superblock inode_size is < 128"));
            }
            size
        } else {
            128
        };

        let bgdt_offset = if block_size == 1024 {
            2 * block_size
        } else {
            block_size
        };

        let mut fs = Self {
            disk,
            device_block_size,
            block_size,
            inodes_per_group,
            inode_size,
            bgdt_offset,
            root_inode: INode {
                inode_number: 0,
                mode: 0,
                size: 0,
                block_pointers: [0; 15],
            },
        };

        let root_inode = fs.read_inode(ROOT_INODE_NUMBER)?;
        if !root_inode.is_directory() {
            return Err(Ext2Error::Corrupt("root inode is not a directory"));
        }
        fs.root_inode = root_inode;
        Ok(fs)
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }

    pub fn root_inode(&self) -> INode {
        self.root_inode
    }

    pub fn find(&self, node: INode, path: &str) -> Result<INode, Ext2Error> {
        if path.starts_with('/') {
            let relative_path = path.trim_start_matches('/');
            if relative_path.is_empty() || relative_path == "." {
                return Ok(self.root_inode);
            }
            return self.root_inode.find(self, relative_path);
        }

        node.find(self, path)
    }

    pub fn read_inode(&self, inode_number: u32) -> Result<INode, Ext2Error> {
        if inode_number == 0 {
            return Err(Ext2Error::InvalidInodeNumber(inode_number));
        }

        let inode_index = inode_number - 1;
        let group = inode_index / self.inodes_per_group;
        let index_in_group = inode_index % self.inodes_per_group;

        let descriptor_offset = self
            .bgdt_offset
            .checked_add(group as usize * BLOCK_GROUP_DESCRIPTOR_SIZE)
            .ok_or(Ext2Error::IntegerOverflow)?;
        let mut descriptor = [0_u8; BLOCK_GROUP_DESCRIPTOR_SIZE];
        let descriptor_block_idx = descriptor_offset / self.block_size;
        let descriptor_offset_in_block = descriptor_offset % self.block_size;
        let mut block = vec![0_u8; self.block_size];
        if descriptor_block_idx > u32::MAX as usize {
            return Err(Ext2Error::IntegerOverflow);
        }
        self.read(descriptor_block_idx as u32, &mut block)?;

        if descriptor_offset_in_block + BLOCK_GROUP_DESCRIPTOR_SIZE <= self.block_size {
            descriptor.copy_from_slice(
                &block[descriptor_offset_in_block
                    ..descriptor_offset_in_block + BLOCK_GROUP_DESCRIPTOR_SIZE],
            );
        } else {
            let first_len = self.block_size - descriptor_offset_in_block;
            descriptor[..first_len].copy_from_slice(&block[descriptor_offset_in_block..]);

            let next_block_idx = descriptor_block_idx
                .checked_add(1)
                .ok_or(Ext2Error::IntegerOverflow)?;
            if next_block_idx > u32::MAX as usize {
                return Err(Ext2Error::IntegerOverflow);
            }
            self.read(next_block_idx as u32, &mut block)?;
            descriptor[first_len..]
                .copy_from_slice(&block[..BLOCK_GROUP_DESCRIPTOR_SIZE - first_len]);
        }
        let inode_table_block = read_le_u32(&descriptor, 8)?;
        if inode_table_block == 0 {
            return Err(Ext2Error::Corrupt("inode table block pointer is zero"));
        }

        let inode_offset = (inode_table_block as usize)
            .checked_mul(self.block_size)
            .and_then(|value| value.checked_add(index_in_group as usize * self.inode_size))
            .ok_or(Ext2Error::IntegerOverflow)?;

        let mut raw_inode = vec![0_u8; self.inode_size];
        let mut inode_copied = 0_usize;
        while inode_copied < raw_inode.len() {
            let absolute_offset = inode_offset
                .checked_add(inode_copied)
                .ok_or(Ext2Error::IntegerOverflow)?;
            let fs_block_idx = absolute_offset / self.block_size;
            let offset_in_block = absolute_offset % self.block_size;
            let copy_len = min(
                raw_inode.len() - inode_copied,
                self.block_size - offset_in_block,
            );
            if fs_block_idx > u32::MAX as usize {
                return Err(Ext2Error::IntegerOverflow);
            }

            self.read(fs_block_idx as u32, &mut block)?;
            raw_inode[inode_copied..inode_copied + copy_len]
                .copy_from_slice(&block[offset_in_block..offset_in_block + copy_len]);
            inode_copied += copy_len;
        }

        let mode = read_le_u16(&raw_inode, 0)?;
        let size = read_le_u32(&raw_inode, 4)?;

        let mut block_pointers = [0_u32; 15];
        let mut entry_offset = 40;
        for pointer in &mut block_pointers {
            *pointer = read_le_u32(&raw_inode, entry_offset)?;
            entry_offset += 4;
        }

        Ok(INode {
            inode_number,
            mode,
            size,
            block_pointers,
        })
    }

    pub fn read(&self, fs_block_idx: u32, out: &mut [u8]) -> Result<(), Ext2Error> {
        if out.len() != self.block_size {
            return Err(Ext2Error::InvalidBufferSize);
        }
        let offset = (fs_block_idx as usize)
            .checked_mul(self.block_size)
            .ok_or(Ext2Error::IntegerOverflow)?;
        let mut copied = 0_usize;
        let mut device_block = vec![0_u8; self.device_block_size];

        while copied < out.len() {
            let absolute_offset = offset
                .checked_add(copied)
                .ok_or(Ext2Error::IntegerOverflow)?;
            let disk_block = absolute_offset / self.device_block_size;
            let offset_in_block = absolute_offset % self.device_block_size;
            let copy_len = min(out.len() - copied, self.device_block_size - offset_in_block);

            let mut buffers: [&mut [u8]; 1] = [&mut device_block];
            self.disk
                .read_blocks(&[disk_block], &mut buffers)
                .map_err(|_| Ext2Error::BlockDevice)?;

            out[copied..copied + copy_len]
                .copy_from_slice(&device_block[offset_in_block..offset_in_block + copy_len]);
            copied += copy_len;
        }

        Ok(())
    }
}

fn read_u8(buffer: &[u8], offset: usize) -> Result<u8, Ext2Error> {
    buffer
        .get(offset)
        .copied()
        .ok_or(Ext2Error::Corrupt("read past end of structure"))
}

fn read_le_u16(buffer: &[u8], offset: usize) -> Result<u16, Ext2Error> {
    let lo = read_u8(buffer, offset)?;
    let hi = read_u8(buffer, offset + 1)?;
    Ok(u16::from_le_bytes([lo, hi]))
}

fn read_le_u32(buffer: &[u8], offset: usize) -> Result<u32, Ext2Error> {
    let b0 = read_u8(buffer, offset)?;
    let b1 = read_u8(buffer, offset + 1)?;
    let b2 = read_u8(buffer, offset + 2)?;
    let b3 = read_u8(buffer, offset + 3)?;
    Ok(u32::from_le_bytes([b0, b1, b2, b3]))
}
