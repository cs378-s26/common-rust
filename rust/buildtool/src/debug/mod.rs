#![allow(non_upper_case_globals)]

use anyhow::Result;
use dwarf::{Context, FunctionInfo, LineInfo};
use gimli::{DwarfSections, EndianSlice, RunTimeEndian, SectionId};
use io::DebugModuleFileWriter;
use object::{Object, ObjectSection};
use std::{borrow::Cow, path::PathBuf};

mod dwarf;
mod io;
mod util;

pub fn gen_debug_module(
    elf_contents: Vec<u8>,
    crate_paths: &Vec<(String, PathBuf)>,
) -> Result<Vec<u8>> {
    let object = object::File::parse(&*elf_contents).unwrap();

    let endian = if object.is_little_endian() {
        RunTimeEndian::Little
    } else {
        RunTimeEndian::Big
    };

    let dwarf_sections = DwarfSections::load(&|id: SectionId| -> Result<Cow<[u8]>> {
        Ok(match object.section_by_name(id.name()) {
            Some(section) => section.uncompressed_data()?,
            None => Cow::Borrowed(&[]),
        })
    })?;

    let dwarf = dwarf_sections.borrow(|section| EndianSlice::new(Cow::as_ref(section), endian));

    let mut iter = dwarf.units();

    let mut writer = DebugModuleFileWriter::new();

    let ctx = Context::parse(&dwarf)?;

    while let Some(header) = iter.next()? {
        let unit = dwarf.unit(header)?;
        let unit = unit.unit_ref(&dwarf);

        let li = LineInfo::parse(unit, crate_paths)?;
        if let Some(line_info) = &li {
            writer.write_line(line_info);
        }

        for func in FunctionInfo::parse(&ctx, unit)? {
            writer.write_function(&func, li.as_ref());
        }
    }

    Ok(writer.write())
}
