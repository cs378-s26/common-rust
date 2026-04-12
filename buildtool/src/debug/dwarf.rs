use std::{mem, path::PathBuf};

use anyhow::{Error, Result};
use gimli::{
    Abbreviation, Attribute, AttributeValue, ColumnType, DW_AT_MIPS_linkage_name,
    DW_AT_abstract_origin, DW_AT_call_column, DW_AT_call_file, DW_AT_call_line, DW_AT_high_pc,
    DW_AT_linkage_name, DW_AT_low_pc, DW_AT_name, DW_AT_ranges, DW_AT_specification,
    DW_TAG_inlined_subroutine, DW_TAG_subprogram, DebugInfoOffset, Dwarf, EntriesRaw, FileEntry,
    LineProgramHeader, RangeListsOffset, Reader, Unit, UnitOffset, UnitRef,
};

type Range = std::ops::Range<u64>;

pub struct Context<'a, R: Reader> {
    pub dwarf: &'a Dwarf<R>,
    units: Vec<(DebugInfoOffset<R::Offset>, Unit<R>)>,
}

pub struct FunctionInfo {
    pub ranges: Vec<Range>,
    pub inlined: Vec<InlinedFunctionInfo>,
    pub name: Option<String>,
}

pub struct LineSequence {
    pub range: Range,
    pub rows: Vec<LineEntry>,
}

pub struct LineEntry {
    pub address: u64,
    pub location: SourceLocation,
}

pub struct LineInfo {
    pub files: Vec<String>,
    pub sequences: Vec<LineSequence>,
}

pub struct InlinedFunctionInfo {
    pub ranges: Vec<Range>,
    pub name: Option<String>,
    pub inlined: Vec<InlinedFunctionInfo>,
    pub location: SourceLocation,
}

pub struct RangeAttributes<R: Reader> {
    pub low_pc: Option<u64>,
    pub high_pc: Option<u64>,
    pub size: Option<u64>,
    pub ranges_offset: Option<RangeListsOffset<<R as Reader>::Offset>>,
}

pub struct SourceLocation {
    pub file: usize,
    pub row: u64,
    pub col: u64,
}

fn convert(range: gimli::Range) -> Range {
    range.begin..range.end
}

pub fn check_range(range: gimli::Range) -> bool {
    range.end >= 0xffffffff80000000 && range.begin < range.end
}

impl Default for SourceLocation {
    fn default() -> Self {
        Self {
            file: usize::MAX,
            row: 0,
            col: 0,
        }
    }
}

impl<'a, R: Reader> Context<'a, R> {
    pub fn parse(dwarf: &'a Dwarf<R>) -> Result<Self> {
        let mut res_units = Vec::new();
        let mut units = dwarf.units();
        while let Some(header) = units.next()? {
            let offset = match header.offset().as_debug_info_offset() {
                Some(offset) => offset,
                None => continue,
            };

            let dw_unit = match dwarf.unit(header) {
                Ok(dw_unit) => dw_unit,
                Err(_) => continue,
            };

            res_units.push((offset, dw_unit));
        }

        Ok(Context {
            dwarf,
            units: res_units,
        })
    }

    pub fn find_offset(&self, offset: DebugInfoOffset<R::Offset>) -> Result<&Unit<R>> {
        match self.units.binary_search_by_key(&offset.0, |unit| unit.0.0) {
            Ok(_) | Err(0) => Err(gimli::Error::NoEntryAtGivenOffset.into()),
            Err(i) => Ok(&self.units[i - 1].1),
        }
    }

    pub fn find_unit(
        &self,
        offset: DebugInfoOffset<R::Offset>,
    ) -> Result<(&Unit<R>, UnitOffset<R::Offset>)> {
        let unit = self.find_offset(offset)?;
        let unit_offset = offset
            .to_unit_offset(&unit.header)
            .ok_or(gimli::Error::NoEntryAtGivenOffset)?;
        Ok((unit, unit_offset))
    }
}

impl<R: Reader> RangeAttributes<R> {
    pub const fn default() -> RangeAttributes<R> {
        RangeAttributes {
            low_pc: None,
            high_pc: None,
            size: None,
            ranges_offset: None,
        }
    }

    pub fn to_vec(&self, unit: UnitRef<R>) -> Result<Vec<Range>> {
        let mut vec = Vec::new();

        let mut add_range = |range: gimli::Range| {
            if range.begin < range.end && check_range(range) {
                vec.push(convert(range));
            }
        };

        if let Some(ranges_offset) = self.ranges_offset {
            let mut range_list = unit.ranges(ranges_offset)?;
            while let Some(range) = range_list.next()? {
                add_range(range);
            }
        } else if let (Some(begin), Some(end)) = (self.low_pc, self.high_pc) {
            add_range(gimli::Range { begin, end });
        } else if let (Some(begin), Some(size)) = (self.low_pc, self.size) {
            let end = begin.wrapping_add(size);
            add_range(gimli::Range { begin, end });
        }
        Ok(vec)
    }
}

impl FunctionInfo {
    fn name_attr<R: Reader>(
        ctx: &Context<R>,
        attr: AttributeValue<R>,
        unit: UnitRef<R>,
        recursion_limit: usize,
    ) -> Result<Option<String>, Error> {
        if recursion_limit == 0 {
            return Ok(None);
        }

        match attr {
            AttributeValue::UnitRef(offset) => Self::name_entry(ctx, unit, offset, recursion_limit),
            AttributeValue::DebugInfoRef(dr) => {
                let (unit, offset) = ctx.find_unit(dr)?;
                let unit = UnitRef::new(ctx.dwarf, unit);
                Self::name_entry(ctx, unit, offset, recursion_limit)
            }
            _ => Ok(None),
        }
    }

    fn name_entry<R: Reader>(
        ctx: &Context<R>,
        unit: UnitRef<R>,
        offset: UnitOffset<R::Offset>,
        recursion_limit: usize,
    ) -> Result<Option<String>, Error> {
        let mut entries = unit.entries_raw(Some(offset))?;
        let abbrev = if let Some(abbrev) = entries.read_abbreviation()? {
            abbrev
        } else {
            return Err(gimli::Error::NoEntryAtGivenOffset.into());
        };

        let mut name = None;
        let mut next = None;

        for spec in abbrev.attributes() {
            let attr = entries.read_attribute(*spec)?;
            match attr.name() {
                DW_AT_linkage_name | DW_AT_MIPS_linkage_name => {
                    if let Ok(val) = unit.attr_string(attr.value()) {
                        return Ok(Some(val.to_string()?.into()));
                    }
                }
                DW_AT_name => {
                    if let Ok(val) = unit.attr_string(attr.value()) {
                        name = Some(val.to_string()?.into());
                    }
                }
                DW_AT_abstract_origin | DW_AT_specification => {
                    next = Some(attr.value());
                }
                _ => {}
            }
        }

        if name.is_some() {
            return Ok(name);
        }

        if let Some(next) = next {
            return Self::name_attr(ctx, next, unit, recursion_limit - 1);
        }

        Ok(None)
    }

    fn parse_common_function_data<R: Reader>(
        ctx: &Context<R>,
        unit: UnitRef<R>,
        attr: &Attribute<R>,
        ranges: &mut RangeAttributes<R>,
        name: &mut Option<String>,
    ) -> Result<()> {
        match attr.name() {
            DW_AT_low_pc => match attr.value() {
                AttributeValue::Addr(val) => ranges.low_pc = Some(val),
                AttributeValue::DebugAddrIndex(index) => {
                    ranges.low_pc = Some(unit.address(index)?);
                }
                _ => {}
            },
            DW_AT_high_pc => match attr.value() {
                AttributeValue::Addr(val) => ranges.high_pc = Some(val),
                AttributeValue::DebugAddrIndex(index) => {
                    ranges.high_pc = Some(unit.address(index)?);
                }
                AttributeValue::Udata(val) => ranges.size = Some(val),
                _ => {}
            },
            DW_AT_ranges => {
                ranges.ranges_offset = unit.attr_ranges_offset(attr.value())?;
            }
            DW_AT_linkage_name | DW_AT_MIPS_linkage_name => {
                if let Ok(val) = unit.attr_string(attr.value()) {
                    *name = Some(val.to_string()?.into());
                }
            }
            DW_AT_name => {
                if name.is_none() {
                    *name = Some(unit.attr_string(attr.value())?.to_string()?.into());
                }
            }
            DW_AT_abstract_origin | DW_AT_specification => {
                if name.is_none() {
                    *name = Self::name_attr(ctx, attr.value(), unit, 16)?;
                }
            }
            _ => {}
        };

        Ok(())
    }

    fn parse_children<R: Reader>(
        ctx: &Context<R>,
        unit: UnitRef<R>,
        entries: &mut EntriesRaw<R>,
        depth: isize,
    ) -> Result<Vec<InlinedFunctionInfo>> {
        let mut result = Vec::new();

        loop {
            let next_depth = entries.next_depth();

            if next_depth <= depth {
                return Ok(result);
            }

            if let Some(abbrev) = entries.read_abbreviation()? {
                match abbrev.tag() {
                    DW_TAG_subprogram => {
                        entries.skip_attributes(abbrev.attributes())?;
                        while entries.next_depth() > depth {
                            if let Some(abbrev) = entries.read_abbreviation()? {
                                entries.skip_attributes(abbrev.attributes())?;
                            }
                        }
                    }
                    DW_TAG_inlined_subroutine => result.push(InlinedFunctionInfo::parse(
                        ctx, unit, entries, abbrev, next_depth,
                    )?),
                    _ => {
                        entries.skip_attributes(abbrev.attributes())?;
                    }
                }
            }
        }
    }

    pub fn parse<R: Reader>(ctx: &Context<R>, unit: UnitRef<R>) -> Result<Vec<FunctionInfo>> {
        let mut entries = unit.entries_raw(None)?;

        let mut result = Vec::new();

        while !entries.is_empty() {
            let depth = entries.next_depth();

            let Some(abbrev) = entries.read_abbreviation()? else {
                continue;
            };

            if abbrev.tag() != DW_TAG_subprogram {
                entries.skip_attributes(abbrev.attributes())?;
                continue;
            }

            let mut ranges: RangeAttributes<R> = RangeAttributes::default();
            let mut name = None;

            for spec in abbrev.attributes() {
                let attr = &entries.read_attribute(*spec)?;
                Self::parse_common_function_data(ctx, unit, attr, &mut ranges, &mut name)?;
            }

            let ranges = ranges.to_vec(unit)?;

            if !ranges.is_empty() {
                result.push(FunctionInfo {
                    ranges,
                    inlined: Self::parse_children(ctx, unit, &mut entries, depth)?,
                    name,
                });
            }
        }

        Ok(result)
    }
}

impl InlinedFunctionInfo {
    fn parse<R: Reader>(
        ctx: &Context<R>,
        unit: UnitRef<R>,
        entries: &mut EntriesRaw<R>,
        abbrev: &Abbreviation,
        depth: isize,
    ) -> Result<InlinedFunctionInfo> {
        let mut ranges = RangeAttributes::default();
        let mut name = None;
        let mut location: SourceLocation = SourceLocation::default();

        for spec in abbrev.attributes() {
            let attr = &entries.read_attribute(*spec)?;
            FunctionInfo::parse_common_function_data(ctx, unit, attr, &mut ranges, &mut name)?;

            match attr.name() {
                DW_AT_call_file => {
                    if let AttributeValue::FileIndex(fi) = attr.value()
                        && (fi > 0 || unit.header.version() >= 5)
                    {
                        location.file = fi as usize;
                    }
                }
                DW_AT_call_line => location.row = attr.udata_value().unwrap_or(0),
                DW_AT_call_column => location.col = attr.udata_value().unwrap_or(0),
                _ => {}
            }
        }

        Ok(InlinedFunctionInfo {
            ranges: ranges.to_vec(unit)?,
            name,
            inlined: FunctionInfo::parse_children(ctx, unit, entries, depth)?,
            location,
        })
    }
}

impl LineInfo {
    fn format_path<R: Reader>(
        dw_unit: UnitRef<R>,
        file: &FileEntry<R, R::Offset>,
        header: &LineProgramHeader<R, R::Offset>,
        crate_paths: &Vec<(String, PathBuf)>,
    ) -> Result<String> {
        let mut path = if let Some(ref comp_dir) = dw_unit.comp_dir {
            comp_dir.to_string_lossy()?.into_owned().into()
        } else {
            PathBuf::new()
        };

        if file.directory_index() != 0
            && let Some(directory) = file.directory(header)
        {
            path.push(dw_unit.attr_string(directory)?.to_string_lossy()?.as_ref());
        }

        path.push(
            dw_unit
                .attr_string(file.path_name())?
                .to_string_lossy()?
                .as_ref(),
        );

        for (crate_name, base) in crate_paths {
            if let Ok(short) = path.strip_prefix(base)
                && let Some(path) = short.to_str()
            {
                return Ok(format!("[{}]:{}", crate_name, path));
            }
        }

        Ok(path
            .to_str()
            .ok_or(Error::msg("failed to convert path to string"))?
            .into())
    }

    pub fn parse<R: Reader>(
        dw_unit: UnitRef<R>,
        crate_paths: &Vec<(String, PathBuf)>,
    ) -> Result<Option<Self>> {
        let Some(ref line_prog) = dw_unit.line_program else {
            return Ok(None);
        };

        let mut sequences = Vec::new();
        let mut sequence_rows = Vec::<LineEntry>::new();
        let mut rows = line_prog.clone().rows();
        while let Some((_, row)) = rows.next_row()? {
            if row.end_sequence() {
                if let Some(start) = sequence_rows.first().map(|x| x.address) {
                    let end = row.address();
                    let mut rows = Vec::new();
                    mem::swap(&mut rows, &mut sequence_rows);
                    let range = gimli::Range { begin: start, end };
                    if start < end && check_range(range) {
                        sequences.push(LineSequence {
                            range: convert(range),
                            rows,
                        });
                    }
                }
                continue;
            }

            let address = row.address();
            let file_index = row.file_index();
            let line = row.line().map(|f| f.get()).unwrap_or(0);
            let column = match row.column() {
                ColumnType::LeftEdge => 0,
                ColumnType::Column(x) => x.get(),
            };

            if let Some(last_row) = sequence_rows.last_mut()
                && last_row.address == address
            {
                last_row.location.file = file_index as usize;
                last_row.location.row = line;
                last_row.location.col = column;
                continue;
            }

            sequence_rows.push(LineEntry {
                address,
                location: SourceLocation {
                    col: column,
                    row: line,
                    file: file_index as usize,
                },
            });
        }

        let mut files = Vec::new();
        let header = rows.header();
        match header.file(0) {
            Some(file) => files.push(Self::format_path(dw_unit, file, header, crate_paths)?),
            None => files.push(String::from("")),
        }
        let mut index = 1;
        while let Some(file) = header.file(index) {
            files.push(Self::format_path(dw_unit, file, header, crate_paths)?);
            index += 1;
        }

        Ok(Some(Self { files, sequences }))
    }
}
