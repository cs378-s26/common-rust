# Kernel Command Line

The kernel may receive command line information from Limine. The kernel has a command line, and each module has one.

## Usage

The way to define command line options is via `struct` or `enum`, and deriving the `CmdlineParsable` trait. Most types are supported.

Options should be added to the relevant struct, with `KernelCmdline` being the root. You are expected to provide default configuration options.

## Syntax

The command line is a comma-separated sequence of key-value pairs. Whitespace (spaces, tabs, newlines) is ignored between tokens.

### Structs

Struct fields are written as `field: value`. Multiple fields are separated by commas.

```
logging: { serial: { enable: true }, fb: { enable: false } }
```

Bitflag fields can be set or cleared directly using the field name (without a colon):

```
{ field }     # set the flag
{ !field }    # clear the flag
```

### Enums

Enum variants are written as either:

```
Variant { field: value, ... }   # struct-like variant
Variant(value, ...)             # tuple-like variant
```

### Booleans

Booleans accept the identifiers `true` and `false`, or a numeric value where `0` is `false` and any non-zero value is `true`.

```
enable: true
enable: false
enable: 1
enable: 0
```

### Integers

Integer fields accept decimal, hexadecimal, and octal literals. Negative values are supported.

| Format      | Example    | Value   |
|-------------|------------|---------|
| Decimal     | `255`      | 255     |
| Hexadecimal | `0xff`     | 255     |
| Octal (0o)  | `0o377`    | 255     |
| Octal (0)   | `0377`     | 255     |
| Negative    | `-42`      | -42     |

Supported integer types: `u8`, `u16`, `u32`, `u64`, `i8`, `i16`, `i32`, `i64`.

### Bitflags

Bitflag fields accept one or more named flags. Prefix a flag with `!` to clear it.

```
flags: FLAG_A | FLAG_B
flags: !FLAG_C
```

Flag names are matched case-insensitively.

## Implementing `CmdlineParsable`

### Derive macro

The easiest way to make a struct or enum parsable is with the derive macro:

```rust
#[derive(CmdlineParsable)]
pub struct MyOptions {
    pub enabled: bool,
    pub level: u32,
}
```

### Manual implementation

You can also implement the trait manually for custom parsing logic. The trait is defined in `parse.rs`:

```rust
pub trait CmdlineParsable {
    fn parse<'a>(&mut self, lexer: &mut CmdlineLexer<'a>) -> Result<(), CmdlineParseError<'a>>;
}
```

Example — the root `KernelCmdline` struct implements this manually:

```rust
impl CmdlineParsable for KernelCmdline {
    fn parse<'a>(&mut self, lexer: &mut CmdlineLexer<'a>) -> Result<(), CmdlineParseError<'a>> {
        lexer.parse_block(CmdlineTokenData::Eof, CmdlineTokenData::Comma, |lexer| {
            let tok = lexer.next_tok()?;
            match tok.unwrap_ident()? {
                "logging" => {
                    lexer.expect(CmdlineTokenData::Colon)?;
                    self.logging.parse(lexer)
                }
                _ => Err(tok.make_error(CmdlineErrorCode::UnknownFlag(&["logging"]))),
            }
        })
    }
}
```

### Implementing bitflag parsing

To make a bitflags type parsable, implement the `ParsableFlags` marker trait (from `parse.rs`). The `CmdlineParsable` impl is provided automatically:

```rust
impl ParsableFlags for MyFlags {}
```

## Runtime API

These functions are available after `parse_kernel_cmdline()` has been called (typically early in kernel init).

```rust
// Returns a reference to the parsed command line state.
// Falls back to DEFAULT_OPTIONS if parsing failed or no cmdline was provided.
pub fn get_cmdline() -> &'static KernelCmdline

// Returns the raw command line string from Limine, if available.
pub fn get_cmdline_text() -> Option<&'static str>

// Returns the first error encountered during parsing, if any.
pub fn get_cmdline_error() -> Option<&'static CmdlineError>
```

### Error variants

```rust
pub enum CmdlineError {
    NoResponse,            // Limine did not provide a cmdline response
    Utf8Error(Utf8Error),  // The cmdline bytes were not valid UTF-8
    ParseError(CmdlineParseError<'static>), // The cmdline failed to parse
}
```

When a parse error occurs, the kernel state is reset to `DEFAULT_OPTIONS`. The error is still retrievable via `get_cmdline_error()`.

## Tokens

The lexer recognises the following tokens. Unrecognised characters produce a `BadToken` error.

| Token        | Example          |
|--------------|------------------|
| Identifier   | `hello`, `_foo`  |
| Number       | `42`, `0xff`     |
| Comma        | `,`              |
| Colon        | `:`              |
| OpenBrace    | `{`              |
| ClosedBrace  | `}`              |
| OpenParen    | `(`              |
| ClosedParen  | `)`              |
| Not          | `!`              |
| Or           | `\|`             |

## Parse errors

`CmdlineParseError` wraps an error code and the byte range in the input string where the error occurred. Errors are reported to the user via `Display` with a human-readable message and the position.

| Error code                   | Meaning                                             |
|------------------------------|-----------------------------------------------------|
| `ExpectedToken`              | A specific token was required but a different one was found |
| `UnknownField`               | A struct field name was not recognised              |
| `UnknownFlagField`           | A flag field name was not recognised                |
| `UnknownEnumerator`          | An enum variant name was not recognised             |
| `UnknownFlag`                | A bitflag name was not recognised                   |
| `BadToken`                   | The lexer encountered an unrecognised character     |
| `BadBoolean`                 | A token could not be interpreted as a boolean       |
| `BadInt`                     | A token could not be interpreted as an integer      |

## Example command line

```
logging: { serial: { enable: true }, fb: { enable: false } }
```

This enables serial logging and disables framebuffer logging.

