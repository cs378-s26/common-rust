use crate::{
    print::kprintln,
    arch::{Arch, ArchTrait},
    virtual_memory::{PagingOptions, VirtualMemoryAllocation},
    fs::vfs::FileTrait,
};

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

    pub const E_VERSION: u32 = 1; // Current version.

    pub const E_EHSIZE: u16 = 64;
    pub const E_PHENTSIZE: u16 = 56; // If supporting more types, this could be 0.
}

mod ph_constants {
    pub const PT_LOAD: u32 = 1;
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
        // TODO: is there a better way to do this without unsafe?
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

pub struct Elf;

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
    PHFileReadError,
    PHInvalidMemSize,
    PHUnsupportedType,
}

impl Elf {
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
        // TODO: AArch64-specific checks? Something about flags. Not quite sure what you guys support.
        Ok(())
    }

    fn load(file: &dyn FileTrait) -> Result<u64, ElfError> {
        const EHSIZE: usize = eh_constants::E_EHSIZE as usize;
        let first_page_vm = VirtualMemoryAllocation::new(
            Arch::get_address_space(),
            None,
            Arch::PAGE_SIZE,
            None,
            PagingOptions::PRESENT
                | PagingOptions::WRITABLE,
            true,
        ).unwrap();
        file.read_page(first_page_vm.base as *mut u8, 0).map_err(|_| ElfError::EHFileReadError)?;
        // TODO: can you do this without unsafe?
        let header_buffer = unsafe { core::slice::from_raw_parts(first_page_vm.base as *const u8, EHSIZE) };

        let header = ElfHeader::parse(&header_buffer);
        Self::validate_elf_header(&header)?;

        let phoff = header.e_phoff as usize;
        let phnum = header.e_phnum as usize;
        let entry = header.e_entry;

        const PHENTSIZE: usize = eh_constants::E_PHENTSIZE as usize;
        let ph_page = first_page_vm;
        let mut ph_page_offset = 0;

        for i in 0..phnum {
            // TODO: maybe read the file in a better way.
            if phoff + i * PHENTSIZE >= ph_page_offset + Arch::PAGE_SIZE {
                // On different page.
                // Round down to page boundary.
                ph_page_offset = (phoff + i * PHENTSIZE) & !(Arch::PAGE_SIZE - 1);
                file.read_page(ph_page.base as *mut u8, ph_page_offset).map_err(|_| ElfError::PHFileReadError)?;
            }
            let size_to_read = PHENTSIZE.min((ph_page_offset + Arch::PAGE_SIZE) - (phoff + i * PHENTSIZE));
            let mut ph_buffer = [0u8; PHENTSIZE];
            ph_buffer[..size_to_read].copy_from_slice(unsafe {
                core::slice::from_raw_parts(
                    (ph_page.base + (phoff + i * PHENTSIZE - ph_page_offset)) as *const u8,
                    size_to_read,
                )
            });
            if size_to_read < PHENTSIZE {
                // Need to read next page.
                ph_page_offset += Arch::PAGE_SIZE;
                file.read_page(ph_page.base as *mut u8, ph_page_offset).map_err(|_| ElfError::PHFileReadError)?;
                ph_buffer[size_to_read..].copy_from_slice(unsafe {
                    core::slice::from_raw_parts(
                        (ph_page.base + (phoff + i * PHENTSIZE - ph_page_offset)) as *const u8,
                        PHENTSIZE - size_to_read,
                    )
                });
            }

            let ph = ProgramHeader::parse(&ph_buffer, 0);

            match ph.p_type {
                ph_constants::PT_LOAD => {
                    let memsz = ph.p_memsz as usize;
                    let filesz = ph.p_filesz as usize;
                    let vaddr = ph.p_vaddr as usize;
                    let offset = ph.p_offset as usize;

                    if memsz < filesz {
                        return Err(ElfError::PHInvalidMemSize);
                    }

                    // kprintln!(
                    //     "Load segment. vaddr: {:#x}, memsz: {:#x}, filesz: {:#x}, offset: {:#x}",
                    //     vaddr,
                    //     memsz,
                    //     filesz,
                    //     offset
                    // );
                    // TODO: mmap this when we can mmap from a file.
                    let segment_vm = VirtualMemoryAllocation::new(
                        Arch::get_address_space(),
                        Some(vaddr),
                        memsz,
                        None,
                        PagingOptions::PRESENT
                            | PagingOptions::WRITABLE,
                        false, // Don't unmap when out of scope.
                    ).unwrap();
                    for page in 0..((filesz + Arch::PAGE_SIZE - 1) / Arch::PAGE_SIZE) {
                        let page_offset = offset + page * Arch::PAGE_SIZE;
                        let page_paddr = segment_vm.base + page * Arch::PAGE_SIZE;
                        file.read_page(page_paddr as *mut u8, page_offset).map_err(|_| ElfError::PHFileReadError)?;
                    }
                    // Zero out rest of memory.
                    if memsz > filesz {
                        let zero_start = segment_vm.base + filesz;
                        let zero_length = memsz - filesz;
                        for i in 0..zero_length {
                            unsafe { *(zero_start as *mut u8).add(i) = 0 };
                        }
                    }
                }
                _ => {
                    let segment_type = ph.p_type;
                    kprintln!("Unsupported segment type: {}", segment_type);
                    return Err(ElfError::PHUnsupportedType);
                }
            }
        }

        drop(ph_page);
        Ok(entry)
    }
}

// TODO: tests once we have a file system.
