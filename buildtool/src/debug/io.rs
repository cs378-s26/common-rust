use super::{
    dwarf::{FunctionInfo, InlinedFunctionInfo, LineInfo, SourceLocation},
    util::IntervalMap,
};
use crate::debug::util::InternStringTable;

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
struct LocationEntry {
    file: usize,
    row: u64,
    col: u64,
}

impl LocationEntry {
    const NULL: LocationEntry = LocationEntry {
        file: usize::MAX,
        row: 0,
        col: 0,
    };
}

struct FunctionEntry {
    inline_parent: usize, // possibly -1
    name: usize,          // index into string table, or -1
    location: LocationEntry,
}

pub struct DebugModuleFileWriter {
    strings: InternStringTable,
    functions: Vec<FunctionEntry>,
    location_search: IntervalMap<u64, Vec<(u64, LocationEntry)>>,
    function_search: IntervalMap<u64, usize>,
}

trait WritableEntry: Sized {
    fn write<T: Fn(usize) -> usize>(&self, str_resolve: &T, out: &mut Vec<u8>);

    fn write_all<T: Fn(usize) -> usize>(vec: &Vec<Self>, str_resolve: &T, out: &mut Vec<u8>) {
        let mut buf = Vec::new();
        for func in vec {
            func.write(str_resolve, &mut buf);
        }

        out.extend_from_slice(&buf.len().to_le_bytes());
        out.extend_from_slice(&buf);
    }
}

trait SearchTableWritable: WritableEntry + Eq + Copy {
    const NULL: Self;
}

impl WritableEntry for LocationEntry {
    fn write<T: Fn(usize) -> usize>(&self, str_resolve: &T, out: &mut Vec<u8>) {
        out.extend_from_slice(&str_resolve(self.file).to_le_bytes());
        out.extend_from_slice(&TryInto::<u32>::try_into(self.row).unwrap().to_le_bytes());
        out.extend_from_slice(&TryInto::<u32>::try_into(self.col).unwrap().to_le_bytes());
    }
}

impl WritableEntry for usize {
    fn write<T: Fn(usize) -> usize>(&self, _str_resolve: &T, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}

impl WritableEntry for FunctionEntry {
    fn write<T: Fn(usize) -> usize>(&self, str_resolve: &T, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.inline_parent.to_le_bytes());
        out.extend_from_slice(&str_resolve(self.name).to_le_bytes());
        self.location.write(str_resolve, out);
    }
}

impl SearchTableWritable for LocationEntry {
    const NULL: LocationEntry = LocationEntry::NULL;
}

impl SearchTableWritable for usize {
    const NULL: usize = usize::MAX;
}

impl DebugModuleFileWriter {
    pub fn new() -> DebugModuleFileWriter {
        DebugModuleFileWriter {
            strings: InternStringTable::new(),
            functions: Vec::new(),
            location_search: IntervalMap::new(),
            function_search: IntervalMap::new(),
        }
    }

    fn filter_ranges<T: SearchTableWritable, U: Iterator<Item = (u64, T)>>(
        mut iter: U,
    ) -> impl Iterator<Item = (u64, T)> {
        fn can_merge<'a, T: SearchTableWritable>(
            current: &'a (u64, T),
            next: &'a (u64, T),
        ) -> Option<&'a (u64, T)> {
            assert!(current.0 <= next.0);

            if current.1 == next.1 {
                return Some(current);
            }

            if current.0 == next.0 {
                if current.1 == T::NULL {
                    return Some(next);
                }

                panic!("duplicate entries at addr {:#x}", current.0);
            }

            None
        }

        gen move {
            let Some(mut prev) = iter.next() else {
                return;
            };

            for ele in iter {
                if let Some(new) = can_merge(&prev, &ele) {
                    prev = *new;
                    continue;
                } else {
                    yield prev;
                    prev = ele;
                }
            }

            yield prev;
        }
    }

    fn write_ranges<T: SearchTableWritable, U: Iterator<Item = (u64, T)>, V: Fn(usize) -> usize>(
        out: &mut Vec<u8>,
        str_resolve: &V,
        iter: U,
    ) {
        let mut buf = Vec::new();

        let mut prev_addr = 0;

        for (addr, inst) in Self::filter_ranges(iter) {
            assert!(prev_addr < addr, "{:#x}, {:#x}", prev_addr, addr);
            prev_addr = addr;
            buf.extend_from_slice(
                &TryInto::<u32>::try_into(addr - 0xffffffff80000000)
                    .unwrap()
                    .to_le_bytes(),
            );

            inst.write(str_resolve, &mut buf);
        }

        out.extend_from_slice(&buf.len().to_le_bytes());
        out.extend_from_slice(&buf);
    }

    pub fn write(&self) -> Vec<u8> {
        let mut res = Vec::new();

        res.extend_from_slice(&0u64.to_le_bytes());

        let str_resolve = self.strings.write(&mut res);

        WritableEntry::write_all(&self.functions, &str_resolve, &mut res);

        Self::write_ranges(&mut res, &str_resolve, gen {
            for (range, value) in self.location_search.iter() {
                for (start, loc) in value {
                    yield (*start, *loc);
                }

                yield (*range.end, LocationEntry::NULL);
            }
        });

        Self::write_ranges(&mut res, &str_resolve, gen {
            for (range, value) in self.function_search.iter() {
                yield (*range.start, *value);
                yield (*range.end, usize::MAX);
            }
        });

        res
    }

    fn format_loc(loc: &SourceLocation, info: Option<&LineInfo>) -> String {
        if let Some(info) = info {
            format!(
                "{}:{}:{}",
                if loc.file == usize::MAX {
                    "???"
                } else {
                    &info.files[loc.file]
                },
                loc.row,
                loc.col
            )
        } else {
            "???".into()
        }
    }

    fn translate_loc<T: FnMut(usize) -> usize>(
        intern: &mut T,
        loc: &SourceLocation,
    ) -> LocationEntry {
        LocationEntry {
            file: intern(loc.file),
            row: loc.row,
            col: loc.col,
        }
    }

    pub fn write_line(&mut self, line_info: &LineInfo) {
        let mut intern = self.strings.lazy_intern_table(&line_info.files);

        for entry in &line_info.sequences {
            let interval = entry.range.clone();

            let result: Vec<_> = entry
                .rows
                .iter()
                .filter_map(|row| {
                    if !interval.contains(&row.address) {
                        eprintln!(
                            "warning: range {:#x}-{:#x} does not contain {:#x} for {}",
                            interval.start,
                            interval.end,
                            row.address,
                            Self::format_loc(&row.location, Some(line_info)),
                        );
                        None
                    } else {
                        Some((row.address, Self::translate_loc(&mut intern, &row.location)))
                    }
                })
                .collect();

            assert!(
                self.location_search.insert(&interval, result),
                "attempting to insert duplicate range"
            );
        }
    }

    fn write_inlined<T: FnMut(usize) -> usize>(
        &mut self,
        intern: &mut T,
        expected_parent: usize,
        func: &InlinedFunctionInfo,
    ) {
        let fn_id = {
            let id = self.functions.len();

            self.functions.push(FunctionEntry {
                inline_parent: usize::MAX,
                name: func
                    .name
                    .as_ref()
                    .map(|f| self.strings.intern(f))
                    .unwrap_or(usize::MAX),
                location: Self::translate_loc(intern, &func.location),
            });

            id
        };

        for range in &func.ranges {
            let Some((parent_range, parent_id)) = self.function_search.get(&range.start) else {
                panic!("inlined function range is not contained within any other function")
            };

            assert!(
                *parent_id == expected_parent,
                "inlined function range is not contained within parent range, expected {} got {}",
                expected_parent,
                *parent_id
            );

            assert!(
                *parent_range.start <= range.start && range.end <= *parent_range.end,
                "inlined function range is not contained within parent range"
            );

            let left_part = *parent_range.start..range.start;
            let right_part = range.end..*parent_range.end;
            let parent_id = *parent_id;

            self.function_search.remove(range.start);

            if !left_part.is_empty() {
                assert!(self.function_search.insert(&left_part, parent_id));
            }

            if !right_part.is_empty() {
                assert!(self.function_search.insert(&right_part, parent_id));
            }

            assert!(self.function_search.insert(range, fn_id));
        }

        for func in &func.inlined {
            self.write_inlined(intern, fn_id, func);
        }
    }

    pub fn write_function(&mut self, func: &FunctionInfo, info: Option<&LineInfo>) {
        let fn_id = {
            let id = self.functions.len();

            self.functions.push(FunctionEntry {
                inline_parent: usize::MAX,
                name: func
                    .name
                    .as_ref()
                    .map(|f| self.strings.intern(f))
                    .unwrap_or(usize::MAX),
                location: LocationEntry::NULL,
            });

            id
        };

        for range in &func.ranges {
            assert!(
                self.function_search.insert(range, fn_id),
                "attempting to insert duplicate range"
            );
        }

        if let Some(info) = info {
            // TODO: this is really bad practice but refactoring is too hard
            let definitely_not_self = unsafe {
                let raw: *mut Self = self;
                &mut *raw
            };

            let mut intern = self.strings.lazy_intern_table(&info.files);

            for ele in &func.inlined {
                definitely_not_self.write_inlined(&mut intern, fn_id, ele);
            }
        } else {
            for ele in &func.inlined {
                self.write_inlined(&mut |_| usize::MAX, fn_id, ele);
            }
        }
    }
}
