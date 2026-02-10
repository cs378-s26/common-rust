use bitflags::Flags;

use super::{CmdlineErrorCode, CmdlineLexer, CmdlineParseError, CmdlineTokenData};

pub trait CmdlineParsable {
    fn parse<'a>(&mut self, lexer: &mut CmdlineLexer<'a>) -> Result<(), CmdlineParseError<'a>>;
}

// primitive impls

impl CmdlineParsable for bool {
    fn parse<'a>(&mut self, lexer: &mut CmdlineLexer<'a>) -> Result<(), CmdlineParseError<'a>> {
        let tok = lexer.next()?;

        match tok.0 {
            CmdlineTokenData::Identifier("true") => *self = true,
            CmdlineTokenData::Identifier("false") => *self = false,
            CmdlineTokenData::Number(0) => *self = false,
            CmdlineTokenData::Number(_) => *self = true,
            _ => return Err(tok.make_error(CmdlineErrorCode::BadBoolean(tok.0))),
        }

        Ok(())
    }
}

pub trait ParsableFlags: Flags + Copy {}

impl<T: ParsableFlags> CmdlineParsable for T {
    fn parse<'a>(&mut self, lexer: &mut CmdlineLexer<'a>) -> Result<(), CmdlineParseError<'a>> {
        loop {
            let neg = lexer.peek().0 == CmdlineTokenData::Not;

            if neg {
                lexer.next()?;
            }

            let id_tok = lexer.next()?;
            let id = id_tok.unwrap_ident()?;
            let Some(item) = T::FLAGS.iter().find(|f| f.name().eq_ignore_ascii_case(id)) else {
                // TODO
                return Err(id_tok.make_error(CmdlineErrorCode::UnknownFlag(&[])));
            };

            if neg {
                self.remove(*item.value());
            } else {
                self.insert(*item.value());
            }
        }
    }
}

macro impl_int_parsable($int_type:ident) {
    impl CmdlineParsable for $int_type {
        fn parse<'a>(&mut self, lexer: &mut CmdlineLexer<'a>) -> Result<(), CmdlineParseError<'a>> {
            let tok = lexer.next()?;

            match tok.0 {
                CmdlineTokenData::Number(x) => {
                    *self = match x.try_into() {
                        Ok(x) => x,
                        Err(_) => return Err(tok.make_error(CmdlineErrorCode::BadInt(tok.0))),
                    }
                }
                _ => return Err(tok.make_error(CmdlineErrorCode::BadInt(tok.0))),
            }

            Ok(())
        }
    }
}

impl_int_parsable!(u8);
impl_int_parsable!(u16);
impl_int_parsable!(u32);
impl_int_parsable!(u64);
impl_int_parsable!(i8);
impl_int_parsable!(i16);
impl_int_parsable!(i32);
impl_int_parsable!(i64);
