use addr2line::{
    self, Context,
    gimli::{self, Dwarf, EndianSlice, RunTimeEndian},
};
use anyhow::{Result, bail};
use object::{Object, ObjectSection};
use std::{
    collections::BTreeMap,
    io::{Cursor, Seek, Write},
    path::PathBuf,
    str::FromStr,
};

const BASE_ADDRESS: u64 = 0xffffffff80000000;

pub fn gen_debug_module(
    elf_contents: Vec<u8>,
    crate_paths: &Vec<(String, PathBuf)>,
) -> Result<Vec<u8>> {
    // Input
    let input = object::File::parse(&*elf_contents)?;
    let load_section =
        |id: gimli::SectionId| -> Result<EndianSlice<RunTimeEndian>, Box<dyn std::error::Error>> {
            // I have no idea how this code works...
            let data = input
                .section_by_name(id.name())
                .and_then(|section| section.uncompressed_data().ok())
                .unwrap_or_default();
            let data = Box::leak(data.into_owned().into_boxed_slice());
            Ok(EndianSlice::new(data, RunTimeEndian::Little))
        };
    let dwarf = Dwarf::load(load_section).unwrap();
    let context = Context::from_dwarf(dwarf)?;

    // Gather metadata
    let mut string_to_index: BTreeMap<String, u16> = BTreeMap::new();
    let mut string_list = vec![];
    let mut string_count = 0u16;
    let mut instruction_list = vec![];
    for location in context.find_location_range(0, u64::MAX)? {
        if location.0 < BASE_ADDRESS {
            continue;
        }
        let file_name = location.2.file.unwrap_or_default().to_string();
        let function = context
            .find_frames(location.0)
            .skip_all_loads()?
            .next()?
            .unwrap()
            .function;
        let line = location.2.line.unwrap_or_default() as u16;
        let column = location.2.column.unwrap_or_default() as u16;
        let function_name: String;
        if function.is_none() {
            function_name = "?unknown?".to_string();
        } else {
            function_name = function.unwrap().demangle()?.to_string();
        }
        let file_index = match string_to_index.get(&file_name) {
            Some(i) => i.clone(),
            None => {
                string_to_index.insert(file_name.clone(), string_count);
                string_list.push(file_name);
                string_count += 1;
                string_count - 1
            }
        };
        let function_index = match string_to_index.get(&function_name) {
            Some(i) => i.clone(),
            None => {
                string_to_index.insert(function_name.clone(), string_count);
                string_list.push(function_name);
                string_count += 1;
                string_count - 1
            }
        };
        instruction_list.push(Instruction {
            address: (location.0 - BASE_ADDRESS) as u32,
            file_index,
            function_index,
            line,
            column,
        });
    }

    // String prefix reduction (by crate)
    let mut new_string_list = vec![];
    for s in &string_list {
        let full_path = PathBuf::from_str(&s)?;
        let canonicalized_full_path = match full_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                new_string_list.push(s.clone());
                continue;
            }
        };
        let mut found = false;
        for (s, p) in crate_paths {
            match full_path.strip_prefix(p) {
                Ok(p) => {
                    found = true;
                    new_string_list.push(format!("{}::{}", s, p.to_string_lossy()));
                    break;
                }
                Err(_) => {}
            };
            match canonicalized_full_path.strip_prefix(p) {
                Ok(p) => {
                    found = true;
                    new_string_list.push(format!("{}::{}", s, p.to_string_lossy()));
                    break;
                }
                Err(_) => {}
            };
        }
        if !found {
            bail!("a valid crate file cannot be matched up '{}'!", s);
        }
    }
    assert!(string_list.len() == new_string_list.len());
    string_list = new_string_list;

    // Deindexing
    let mut deindexing_list = vec![];
    let mut string_table_size: u32 = 0;
    for s in &string_list {
        deindexing_list.push(string_table_size);
        string_table_size += (s.len() as u32) + 2;
    }

    // Output
    let mut output = Cursor::new(Vec::new());

    // Create header
    let deindexing_table_offset = 16u32;
    let string_table_offset = deindexing_table_offset + 4 * (string_count as u32);
    let instruction_table_offset = (string_table_offset + string_table_size).next_multiple_of(4);
    let total_size = instruction_table_offset + 12 * (instruction_list.len() as u32);
    output.write(&deindexing_table_offset.to_le_bytes())?;
    output.write(&string_table_offset.to_le_bytes())?;
    output.write(&instruction_table_offset.to_le_bytes())?;
    output.write(&total_size.to_le_bytes())?;

    // Deindexing table
    output.seek(std::io::SeekFrom::Start(deindexing_table_offset.into()))?;
    for o in deindexing_list {
        output.write(&o.to_le_bytes())?;
    }

    // String table
    output.seek(std::io::SeekFrom::Start(string_table_offset.into()))?;
    for s in string_list {
        output.write(&(s.len() as u16).to_le_bytes())?;
        output.write(s.as_bytes())?;
    }

    // Instruction table
    output.seek(std::io::SeekFrom::Start(instruction_table_offset.into()))?;
    for i in instruction_list {
        output.write(&i.address.to_le_bytes())?;
        output.write(&i.file_index.to_le_bytes())?;
        output.write(&i.function_index.to_le_bytes())?;
        output.write(&i.line.to_le_bytes())?;
        output.write(&i.column.to_le_bytes())?;
    }

    output.flush()?;
    Ok(output.into_inner())
}

struct Instruction {
    address: u32,
    file_index: u16,
    function_index: u16,
    line: u16,
    column: u16,
}
