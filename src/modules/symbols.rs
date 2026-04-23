use crate::symbols::SymbolTable;

pub fn parse(data: &'static [u8]) -> Option<SymbolTable> {
    SymbolTable::parse(data)
}

pub fn try_init(table: SymbolTable) -> bool {
    crate::symbols::try_init_table(table)
}
