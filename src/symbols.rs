use limine::request::ModuleRequest;
use spin::Once;

#[unsafe(link_section = ".limine_requests")]
static SYMBOL_MODULE_REQUEST: ModuleRequest = ModuleRequest::new();

static SYMBOL_TABLE: Once<SymbolTable> = Once::new();

pub struct FunctionSymbol {
    pub name: &'static str,
    pub inline_parent: Option<usize>,
}

struct SymbolTable {
    data: &'static [u8],
    strings_offset: usize,
    strings_len: usize,
    functions_offset: usize,
    functions_count: usize,
    function_search_offset: usize,
    function_search_len: usize,
}

impl SymbolTable {
    fn parse(data: &'static [u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }

        let mut pos = 8; // skip header

        let strings_len = read_usize(data, &mut pos)?;
        let strings_offset = pos;
        pos += strings_len;

        let functions_len = read_usize(data, &mut pos)?;
        let functions_offset = pos;
        let functions_count = functions_len / 32;
        pos += functions_len;

        let location_search_len = read_usize(data, &mut pos)?;
        pos += location_search_len;

        let function_search_len = read_usize(data, &mut pos)?;
        let function_search_offset = pos;

        Some(SymbolTable {
            data,
            strings_offset,
            strings_len,
            functions_offset,
            functions_count,
            function_search_offset,
            function_search_len,
        })
    }

    fn get_string(&self, offset: usize) -> Option<&'static str> {
        if offset == usize::MAX {
            return None;
        }
        let start = self.strings_offset + offset;
        let end = self.strings_offset + self.strings_len;
        if start >= end {
            return None;
        }
        let len = self.data[start..end].iter().position(|&b| b == 0)?;
        core::str::from_utf8(&self.data[start..start + len]).ok()
    }

    fn get_function(&self, idx: usize) -> Option<FunctionSymbol> {
        if idx >= self.functions_count {
            return None;
        }
        let base = self.functions_offset + idx * 32;
        let inline_parent = usize::from_le_bytes(self.data[base..base + 8].try_into().ok()?);
        let name_offset = usize::from_le_bytes(self.data[base + 8..base + 16].try_into().ok()?);
        let name = self.get_string(name_offset)?;
        Some(FunctionSymbol {
            name,
            inline_parent: (inline_parent != usize::MAX).then_some(inline_parent),
        })
    }

    fn lookup(&self, addr: u64) -> Option<FunctionSymbol> {
        const KERNEL_BASE: u64 = 0xffffffff80000000;
        if addr < KERNEL_BASE {
            return None;
        }
        let offset_addr = (addr - KERNEL_BASE) as u32;

        const ENTRY_SIZE: usize = 12; // u32 addr + u64 index
        let num_entries = self.function_search_len / ENTRY_SIZE;

        let mut lo = 0;
        let mut hi = num_entries;
        let mut best = None;

        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let pos = self.function_search_offset + mid * ENTRY_SIZE;
            let entry_addr = u32::from_le_bytes(self.data[pos..pos + 4].try_into().ok()?);

            if entry_addr <= offset_addr {
                let idx = usize::from_le_bytes(self.data[pos + 4..pos + 12].try_into().ok()?);
                if idx != usize::MAX {
                    best = Some(idx);
                }
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        best.and_then(|idx| self.get_function(idx))
    }
}

fn read_usize(data: &[u8], pos: &mut usize) -> Option<usize> {
    let val = usize::from_le_bytes(data.get(*pos..*pos + 8)?.try_into().ok()?);
    *pos += 8;
    Some(val)
}

pub fn init_symbols_from_modules() {
    let response = match SYMBOL_MODULE_REQUEST.get_response() {
        Some(r) => r,
        None => return,
    };

    for module in response.modules() {
        if module
            .path()
            .to_str()
            .is_ok_and(|p| p.contains("kernel_symbols.mod"))
        {
            let data =
                unsafe { core::slice::from_raw_parts(module.addr(), module.size() as usize) };
            if let Some(table) = SymbolTable::parse(data) {
                SYMBOL_TABLE.call_once(|| table);
            }
            return;
        }
    }
}

pub fn lookup_symbol(addr: u64) -> Option<FunctionSymbol> {
    SYMBOL_TABLE.get()?.lookup(addr)
}
