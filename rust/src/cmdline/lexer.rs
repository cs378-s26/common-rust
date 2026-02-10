use core::{
    fmt::{Display, Formatter},
    mem,
    ops::Range,
};

use derive_more::Display;
use logos::{Lexer, Logos};

use super::CmdlineParsable;

fn parse_int(mut str: &str) -> i64 {
    let mut neg = false;
    if str.starts_with("-") {
        str = &str[1..];
        neg = true;
    }

    let res;

    if let Some(str) = str.strip_prefix("0x") {
        res = i64::from_str_radix(str, 16).unwrap();
    } else if let Some(str) = str.strip_prefix("0o") {
        res = i64::from_str_radix(str, 8).unwrap();
    } else if let Some(str) = str.strip_prefix("0") {
        if str.is_empty() {
            res = 0;
        } else {
            res = i64::from_str_radix(str, 8).unwrap();
        }
    } else {
        res = str.parse::<i64>().unwrap();
    }

    if neg { -res } else { res }
}

#[derive(Logos, Debug, PartialEq, Clone, Copy, Display)]
#[logos(skip r"[ \t\n\f]+")]
pub enum CmdlineTokenData<'a> {
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token("{")]
    OpenBrace,
    #[token("}")]
    ClosedBrace,
    #[token("!")]
    Not,
    #[token("|")]
    Or,
    #[token("(")]
    OpenParen,
    #[token(")")]
    ClosedParen,
    #[regex("[a-zA-Z_][a-zA-Z0-9_]*")]
    Identifier(&'a str),
    #[regex("-?([1-9][0-9]*|0[0-7]*|0o[0-7]+|0x[0-9a-fA-F]+)", |lex| parse_int(lex.slice()))]
    Number(i64),
    Eof,
}

#[derive(Debug, PartialEq)]
pub enum CmdlineErrorCode<'a> {
    ExpectedToken {
        actual: CmdlineTokenData<'a>,
        expected: CmdlineTokenData<'static>,
    },
    UnknownField(&'static [&'static str]),
    UnknownFlagField(&'static [&'static str]),
    UnknownEnumerator(&'static [&'static str]),
    UnknownFlag(&'static [&'static str]),
    BadToken,
    BadBoolean(CmdlineTokenData<'a>),
    BadInt(CmdlineTokenData<'a>),
}

#[derive(Debug)]
pub struct CmdlineToken<'a>(pub CmdlineTokenData<'a>, pub Range<usize>);

#[derive(Debug)]
pub struct CmdlineParseError<'a>(pub CmdlineErrorCode<'a>, pub Range<usize>);

impl<'a> Display for CmdlineParseError<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            CmdlineErrorCode::ExpectedToken { actual, expected } => {
                write!(f, "expected token {}, but got {}", expected, actual)?
            }
            CmdlineErrorCode::UnknownField(items) => {
                write!(f, "unknown field; options: {:?}", items)?
            }
            CmdlineErrorCode::UnknownFlagField(items) => {
                write!(f, "unknown flag field; options: {:?}", items)?
            }
            CmdlineErrorCode::UnknownEnumerator(items) => {
                write!(f, "unknown enumerator; options: {:?}", items)?
            }
            CmdlineErrorCode::UnknownFlag(items) => {
                write!(f, "unknown bit flag; options: {:?}", items)?
            }
            CmdlineErrorCode::BadToken => f.write_str("bad token")?,
            CmdlineErrorCode::BadBoolean(tok) => write!(f, "bad boolean token: {} ", tok)?,
            CmdlineErrorCode::BadInt(tok) => write!(f, "bad int token: {} ", tok)?,
        };

        write!(f, " at {:?}", self.1)
    }
}

pub struct CmdlineLexer<'a> {
    lexer: Lexer<'a, CmdlineTokenData<'a>>,
    current: CmdlineToken<'a>,
}

impl<'a> CmdlineToken<'a> {
    pub fn unwrap_ident(&self) -> Result<&'a str, CmdlineParseError<'a>> {
        let CmdlineTokenData::Identifier(id) = self.0 else {
            return Err(CmdlineParseError(
                CmdlineErrorCode::ExpectedToken {
                    actual: self.0,
                    expected: CmdlineTokenData::Identifier("*"),
                },
                self.1.clone(),
            ));
        };

        Ok(id)
    }

    pub fn make_error(&self, err_code: CmdlineErrorCode<'a>) -> CmdlineParseError<'a> {
        CmdlineParseError(err_code, self.1.clone())
    }
}

impl<'a> CmdlineLexer<'a> {
    fn lex(
        lexer: &mut Lexer<'a, CmdlineTokenData<'a>>,
    ) -> Result<CmdlineToken<'a>, CmdlineParseError<'a>> {
        match lexer.next() {
            Some(Ok(x)) => Ok(CmdlineToken(x, lexer.span())),
            Some(Err(_)) => Err(CmdlineParseError(CmdlineErrorCode::BadToken, lexer.span())),
            None => Ok(CmdlineToken(CmdlineTokenData::Eof, lexer.span())),
        }
    }

    pub fn new(data: &'a str) -> Result<CmdlineLexer<'a>, CmdlineParseError<'a>> {
        let mut lexer = CmdlineTokenData::lexer(data);
        let tok = Self::lex(&mut lexer)?;

        Ok(CmdlineLexer {
            lexer,
            current: tok,
        })
    }

    pub fn parse<T: CmdlineParsable>(
        data: &'a str,
        out: &mut T,
    ) -> Result<(), CmdlineParseError<'a>> {
        let mut lexer = CmdlineLexer::new(data)?;
        out.parse(&mut lexer)?;
        Ok(())
    }

    pub fn next(&mut self) -> Result<CmdlineToken<'a>, CmdlineParseError<'a>> {
        let mut tok;

        match self.lexer.next() {
            Some(Ok(x)) => {
                tok = CmdlineToken(x, self.lexer.span());
            }
            Some(Err(_)) => {
                return Err(CmdlineParseError(
                    CmdlineErrorCode::BadToken,
                    self.lexer.span(),
                ));
            }
            None => tok = CmdlineToken(CmdlineTokenData::Eof, self.lexer.span()),
        }

        mem::swap(&mut self.current, &mut tok);

        Ok(tok)
    }

    pub fn peek(&self) -> &CmdlineToken<'a> {
        &self.current
    }

    pub fn expect(&mut self, tok: CmdlineTokenData<'static>) -> Result<(), CmdlineParseError<'a>> {
        let CmdlineToken(data, range) = self.next()?;

        if data != tok {
            Err(CmdlineParseError(
                CmdlineErrorCode::ExpectedToken {
                    actual: data,
                    expected: tok,
                },
                range,
            ))
        } else {
            Ok(())
        }
    }

    pub fn parse_block<T: FnMut(&mut Self) -> Result<(), CmdlineParseError<'a>>>(
        &mut self,
        end_tok: CmdlineTokenData<'static>,
        delimiter: CmdlineTokenData<'static>,
        mut handler: T,
    ) -> Result<(), CmdlineParseError<'a>> {
        if self.peek().0 != end_tok {
            loop {
                handler(self)?;

                if self.peek().0 != delimiter {
                    break;
                }

                self.next()?;
            }
        }

        self.expect(end_tok)?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_int_decimal() {
        assert_eq!(parse_int("123"), 123);
        assert_eq!(parse_int("-123"), -123);
    }

    #[test]
    fn test_parse_int_hex() {
        assert_eq!(parse_int("0x1a3"), 0x1a3);
        assert_eq!(parse_int("-0x1a3"), -0x1a3);
    }

    #[test]
    fn test_parse_int_octal() {
        assert_eq!(parse_int("075"), 0o75);
        assert_eq!(parse_int("-075"), -0o75);
    }

    #[test]
    fn test_parse_int_zero() {
        assert_eq!(parse_int("0"), 0);
        assert_eq!(parse_int("-0"), 0);
    }

    #[test]
    fn test_cmdline_tokenizer_identifiers() {
        let data = "hello world _underscore identifier";
        let mut lexer = CmdlineLexer::new(data).unwrap();

        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("hello")
        );
        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("world")
        );
        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("_underscore")
        );
        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("identifier")
        );
    }

    #[test]
    fn test_cmdline_tokenizer_numbers() {
        let data = "123 0x1a3 075 -42";
        let mut lexer = CmdlineLexer::new(data).unwrap();

        assert_eq!(lexer.next().unwrap().0, CmdlineTokenData::Number(123));
        assert_eq!(lexer.next().unwrap().0, CmdlineTokenData::Number(0x1a3));
        assert_eq!(lexer.next().unwrap().0, CmdlineTokenData::Number(0o75));
        assert_eq!(lexer.next().unwrap().0, CmdlineTokenData::Number(-42));
    }

    #[test]
    fn test_cmdline_tokenizer_commas_and_colons() {
        let data = "cmd1, cmd2:cmd3";
        let mut lexer = CmdlineLexer::new(data).unwrap();

        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("cmd1")
        );
        assert_eq!(lexer.next().unwrap().0, CmdlineTokenData::Comma);
        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("cmd2")
        );
        assert_eq!(lexer.next().unwrap().0, CmdlineTokenData::Colon);
        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("cmd3")
        );
    }

    #[test]
    fn test_cmdline_tokenizer_braces() {
        let data = "{cmd1, cmd2}";
        let mut lexer = CmdlineLexer::new(data).unwrap();

        assert_eq!(lexer.next().unwrap().0, CmdlineTokenData::OpenBrace);
        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("cmd1")
        );
        assert_eq!(lexer.next().unwrap().0, CmdlineTokenData::Comma);
        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("cmd2")
        );
        assert_eq!(lexer.next().unwrap().0, CmdlineTokenData::ClosedBrace);
    }

    #[test]
    fn test_cmdline_tokenizer_invalid_token() {
        let data = "cmd1 @ cmd2";
        let mut lexer = CmdlineLexer::new(data).unwrap();

        assert_eq!(lexer.next().unwrap_err().0, CmdlineErrorCode::BadToken);
    }

    #[test]
    fn test_expect_valid_token() {
        let data = "cmd1 : cmd2";
        let mut lexer = CmdlineLexer::new(data).unwrap();

        lexer.expect(CmdlineTokenData::Colon).unwrap();

        assert_eq!(
            lexer.next().unwrap().0,
            CmdlineTokenData::Identifier("cmd2")
        );
    }

    #[test]
    fn test_expect_invalid_token() {
        let data = "cmd1 : cmd2";
        let mut lexer = CmdlineLexer::new(data).unwrap();

        assert_eq!(
            lexer.expect(CmdlineTokenData::Comma).unwrap_err().0,
            CmdlineErrorCode::ExpectedToken {
                actual: CmdlineTokenData::Colon,
                expected: CmdlineTokenData::Comma
            }
        );
    }

    #[test]
    fn test_parse_block_with_delimiter() {
        let data = "{cmd1, cmd2, cmd3}";
        let mut lexer = CmdlineLexer::new(data).unwrap();

        lexer
            .parse_block(
                CmdlineTokenData::ClosedBrace,
                CmdlineTokenData::Comma,
                |lexer| {
                    let ident = lexer.next().unwrap();
                    assert!(matches!(ident.0, CmdlineTokenData::Identifier(_)));
                    Ok(())
                },
            )
            .unwrap();
    }

    #[test]
    fn test_parse_block_end_token() {
        let data = "{cmd1 cmd2}";
        let mut lexer = CmdlineLexer::new(data).unwrap();

        lexer
            .parse_block(
                CmdlineTokenData::ClosedBrace,
                CmdlineTokenData::Comma,
                |lexer| {
                    let ident = lexer.next().unwrap();
                    assert!(matches!(ident.0, CmdlineTokenData::Identifier(_)));
                    Ok(())
                },
            )
            .unwrap();
    }
}
