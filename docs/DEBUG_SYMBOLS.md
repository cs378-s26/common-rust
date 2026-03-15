# Debug Symbols
The standard way of expressing debug symbols in ELF is through DWARF.
However, DWARF is a complex format to parse, and we would rather not
have this logic inside of the kernel. So we opt to parse DWARF at
build time, and convert it to the following format, making it easy for
the kernel to gather information about debug symbols.

## Core Concepts
There are 3 tables: the deindexing table, the string table, and the
instruction table. Their offsets are described in the file's header.
When the debug symbols for an address are wanted, the instruction
table is binary searched for that specific address's record.

The instruction record describes various pieces of data, such as the
line, column, file name, and function name associated with that
instruction. Storing the names of the file and function in each
instruction can be ineffecient, so we opt to store a reference to
another table, the string table.

The records in the string table hold strings along with their length
in a linear order. Naturally, the instruction record would hold
offsets in the string table.

However, since the string table itself can get large, the instruction
record would need to hold 4 bytes of information per string it wants a
reference to. To optimize this, we create the deindexing table. The
instruction record holds an index into the deindexing table, which
then converts a given index into an offset. That way, each instruction
record only has to carry 2 bytes of information per string.

## Header
| Field Name                 | Offset | Size | Description                                                |
|----------------------------|--------|------|------------------------------------------------------------|
| `deindexing_table_offset`  | 0      | 4    | The offset (in bytes) of the deindexing table.             |
| `string_table_offset `     | 4      | 4    | The offset (in bytes) of the string table.                 |
| `instruction_table_offset` | 8      | 4    | The offset (in bytes) of the instruction table.            |
| `total_size`               | 12     | 4    | Total size of this file (in bytes) (including the header). |

All table offsets should be 4 byte aligned.

## Deindexing Table
Each deindexing record is 4 bytes, with one field, the `offset`. For a
given index `i` (0-indexed), here is the description of the record.

| Field Name | Offset | Size | Description                                                  |
|------------|--------|------|--------------------------------------------------------------|
| `offset`   | 4*i    | 4    | The offset in the string table for the corresponding string. |

## String Table
A record in the string table has a dynamic size, so the offset of a
record cannot be determined without first looping through the previous
elements (or looking at the deindexing table). Here are the fields of
a string record for a string of size `n`.

| Field Name | Size | Description                               |
|------------|------|-------------------------------------------|
| `length`   | 2    | Length of string (equal to `n`).          |
| `string`   | n    | The string as bytes, not null-terminated. |

## Instruction Table
A record in the instruction table is 12 bytes. The records should be
sorted by `address`.

| Field Name       | Offset | Size | Description                                                  |
|------------------|--------|------|--------------------------------------------------------------|
| `address`        | 0      | 4    | Address of instruction, subtracted by `0xffffffff80000000`.  |
| `file_index`     | 4      | 2    | The index of the file name in the deindexing table.          |
| `function_index` | 6      | 2    | The index of the function name in the deindexing table.      |
| `line`           | 8      | 2    | The line number corresponding to this instruction address.   |
| `column`         | 10     | 2    | The column number corresponding to this instruction address. |
