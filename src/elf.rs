use alloc::sync::Arc;
use crate::print::kprintln;

use crate::{Arch, ArchTrait, fs::vfs::VNode, process::Process};

mod eh_constants {
    pub const EI_MAG: [u8; 4] = [0x7f, b'E', b'L', b'F'];
    pub const EI_CLASS_64: u8 = 2; // 64-bit.
    pub const EI_DATA: u8 = 1; // Little-endian.
    pub const EI_VERSION: u8 = 1; // Current version.
    pub const EI_OSABI_SYSTEM_V: u8 = 0; // System V ABI.

    pub const E_TYPE_EXECUTABLE: u16 = 2;

    #[cfg(target_arch = "x86_64")]
    pub const E_MACHINE: u16 = 0x3E;
    #[cfg(target_arch = "aarch64")]
    pub const E_MACHINE: u16 = 0xB7;
    #[cfg(target_arch = "aarch64")]
    pub const E_FLAGS: u32 = 0;

    pub const E_VERSION: u32 = 1; // Current version.

    pub const E_EHSIZE: u16 = 64;
    pub const E_PHENTSIZE: u16 = 56; // If supporting more types, this could be 0.
}

mod ph_constants {
    pub const PT_LOAD: u32 = 1;
    pub const PT_NOTE: u32 = 4;
    pub const PT_TLS: u32 = 7;
    pub const PT_GNU_STACK: u32 = 0x6474e551;
    pub const PT_GNU_RELRO: u32 = 0x6474e552;
    pub const PT_GNU_PROPERTY: u32 = 0x6474e553;
    // TODO: handle more? What else do we want?
}

#[repr(C, packed)]
struct ElfHeader {
    ei_mag: [u8; 4],
    ei_class: u8,
    ei_data: u8,
    ei_version: u8,
    ei_osabi: u8,
    ei_abiversion: u8,
    ei_pad: [u8; 7],

    e_type: u16, // 1 = relocatable, 2 = executable, 3 = shared, 4 = core.
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C, packed)]
struct ProgramHeader {
    p_type: u32, // 1 = load, 2 = dynamic, 3 = interp, 4 = note, etc.
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

struct ParseLittleEndian;
impl ParseLittleEndian {
    fn parse_u16(buffer: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes([buffer[offset], buffer[offset + 1]])
    }

    fn parse_u32(buffer: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([
            buffer[offset],
            buffer[offset + 1],
            buffer[offset + 2],
            buffer[offset + 3],
        ])
    }

    fn parse_u64(buffer: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes([
            buffer[offset],
            buffer[offset + 1],
            buffer[offset + 2],
            buffer[offset + 3],
            buffer[offset + 4],
            buffer[offset + 5],
            buffer[offset + 6],
            buffer[offset + 7],
        ])
    }
}

impl ElfHeader {
    fn parse(buffer: &[u8]) -> Self {
        ElfHeader {
            ei_mag: [buffer[0], buffer[1], buffer[2], buffer[3]],
            ei_class: buffer[4],
            ei_data: buffer[5],
            ei_version: buffer[6],
            ei_osabi: buffer[7],
            ei_abiversion: buffer[8],
            ei_pad: [0u8; 7],

            e_type: ParseLittleEndian::parse_u16(buffer, 16),
            e_machine: ParseLittleEndian::parse_u16(buffer, 18),
            e_version: ParseLittleEndian::parse_u32(buffer, 20),
            e_entry: ParseLittleEndian::parse_u64(buffer, 24),
            e_phoff: ParseLittleEndian::parse_u64(buffer, 32),
            e_shoff: ParseLittleEndian::parse_u64(buffer, 40),
            e_flags: ParseLittleEndian::parse_u32(buffer, 48),
            e_ehsize: ParseLittleEndian::parse_u16(buffer, 52),
            e_phentsize: ParseLittleEndian::parse_u16(buffer, 54),
            e_phnum: ParseLittleEndian::parse_u16(buffer, 56),
            e_shentsize: ParseLittleEndian::parse_u16(buffer, 58),
            e_shnum: ParseLittleEndian::parse_u16(buffer, 60),
            e_shstrndx: ParseLittleEndian::parse_u16(buffer, 62),
        }
    }
}

impl ProgramHeader {
    fn parse(buffer: &[u8], offset: usize) -> Self {
        ProgramHeader {
            p_type: ParseLittleEndian::parse_u32(buffer, offset),
            p_flags: ParseLittleEndian::parse_u32(buffer, offset + 4),
            p_offset: ParseLittleEndian::parse_u64(buffer, offset + 8),
            p_vaddr: ParseLittleEndian::parse_u64(buffer, offset + 16),
            p_paddr: ParseLittleEndian::parse_u64(buffer, offset + 24),
            p_filesz: ParseLittleEndian::parse_u64(buffer, offset + 32),
            p_memsz: ParseLittleEndian::parse_u64(buffer, offset + 40),
            p_align: ParseLittleEndian::parse_u64(buffer, offset + 48),
        }
    }
}

#[derive(Debug)]
pub struct ElfLoader;

#[derive(Debug)]
pub enum ElfError {
    EHFileReadError,
    EHInvalidMagic,
    EHUnsupportedClass,
    EHUnsupportedDataEncoding,
    EHUnsupportedVersion,
    EHUnsupportedOsAbi,
    EHUnsupportedType,
    EHUnsupportedMachine,
    EHInvalidElfHeaderSize,
    EHInvalidProgramHeaderSize,
    EHInvalidProgramHeader,
    EHInvalidFlags,
    PHFileReadError,
    PHInvalidMemSize,
    PHUnsupportedType,
    MmapError,
    InodeKeyError,
}

impl ElfLoader {
    fn validate_elf_header(header: &ElfHeader) -> Result<(), ElfError> {
        if header.ei_mag != eh_constants::EI_MAG {
            return Err(ElfError::EHInvalidMagic);
        }
        if header.ei_class != eh_constants::EI_CLASS_64 {
            return Err(ElfError::EHUnsupportedClass);
        }
        if header.ei_data != eh_constants::EI_DATA {
            return Err(ElfError::EHUnsupportedDataEncoding);
        }
        if header.ei_version != eh_constants::EI_VERSION {
            return Err(ElfError::EHUnsupportedVersion);
        }
        if header.ei_osabi != eh_constants::EI_OSABI_SYSTEM_V {
            return Err(ElfError::EHUnsupportedOsAbi);
        }
        if header.e_type != eh_constants::E_TYPE_EXECUTABLE {
            return Err(ElfError::EHUnsupportedType);
        }
        if header.e_machine != eh_constants::E_MACHINE {
            return Err(ElfError::EHUnsupportedMachine);
        }
        if header.e_version != eh_constants::E_VERSION {
            return Err(ElfError::EHUnsupportedVersion);
        }
        if header.e_ehsize as usize != core::mem::size_of::<ElfHeader>() {
            return Err(ElfError::EHInvalidElfHeaderSize);
        }
        if header.e_phentsize as usize != core::mem::size_of::<ProgramHeader>() {
            return Err(ElfError::EHInvalidProgramHeaderSize);
        }
        #[cfg(target_arch = "aarch64")]
        if header.e_flags != eh_constants::E_FLAGS {
            return Err(ElfError::EHInvalidFlags);
        }
        Ok(())
    }

    pub fn load(file: Arc<dyn VNode>, process: &Arc<Process>) -> Result<u64, ElfError> {
        const EHSIZE: usize = eh_constants::E_EHSIZE as usize;
        let mut header_buffer = [0u8; EHSIZE];
        let read_size = file
            .read_unaligned(0, &mut header_buffer)
            .map_err(|_| ElfError::EHFileReadError)?;
        if read_size < EHSIZE {
            return Err(ElfError::EHFileReadError);
        }

        let header = ElfHeader::parse(&header_buffer);
        Self::validate_elf_header(&header)?;

        let phoff = header.e_phoff as usize;
        let phnum = header.e_phnum as usize;
        const PHENTSIZE: usize = eh_constants::E_PHENTSIZE as usize;
        for i in 0..phnum {
            // Read program header.
            let mut ph_buffer = [0u8; PHENTSIZE];
            let read_size = file
                .read_unaligned(phoff + i * PHENTSIZE, &mut ph_buffer)
                .map_err(|_| ElfError::PHFileReadError)?;
            if read_size < PHENTSIZE {
                return Err(ElfError::PHFileReadError);
            }
            let ph = ProgramHeader::parse(&ph_buffer, 0);

            let vm = &process.virtual_memory;
            let inode_key = file.get_inode_key().map_err(|_| ElfError::InodeKeyError)?;

            match ph.p_type {
                ph_constants::PT_LOAD => {
                    let memsz = ph.p_memsz as usize;
                    let filesz = ph.p_filesz as usize;
                    let vaddr = ph.p_vaddr as usize;
                    let offset = ph.p_offset as usize;

                    // in reality the check here needs to be that they have the same offset relative to p_align, 
                    // but for simplicity we just need them to be page aligned so we can map their offsets correctly
                    if vaddr % Arch::PAGE_SIZE != offset % Arch::PAGE_SIZE {
                        return Err(ElfError::EHInvalidProgramHeader);
                    }

                    if memsz < filesz {
                        return Err(ElfError::PHInvalidMemSize);
                    }
                    let vaddr_rounded = vaddr & !(Arch::PAGE_SIZE - 1);
                    let offset_rounded = offset & !(Arch::PAGE_SIZE - 1);
                    let padding = offset - offset_rounded;
                    let map_size = memsz + padding;

                    vm.mmap(
                        Some((inode_key, offset_rounded, Some(filesz))),
                        map_size.div_ceil(Arch::PAGE_SIZE) * Arch::PAGE_SIZE, // Round up.
                        false,
                        Some(vaddr_rounded),
                    )
                    .map_err(|_| ElfError::MmapError)?;
                }
                ph_constants::PT_NOTE => {
                    // Parse notes if needed later.
                }
                ph_constants::PT_GNU_STACK => {
                    // TODO: handle stack permissions. Save this somewhere probably.
                    // ph.p_flags has PF_X (1), PF_W (2), PF_R (4) flags.
                }
                ph_constants::PT_GNU_RELRO => {
                    // "Relocation Read-Only."
                    // TODO: handle relocations.
                }
                ph_constants::PT_GNU_PROPERTY => {
                    // TODO: handle other GNU properties.
                    // Stuff about CPU/ABI/security or something.
                }
                _ => {
                    // TODO: handle other types. Ignore for now. Uncomment for type.
                    let segment_type = ph.p_type;
                    kprintln!("Unsupported segment type: {}", segment_type);
                    return Err(ElfError::PHUnsupportedType);
                }
            }
        }

        Ok(header.e_entry)
    }
}
