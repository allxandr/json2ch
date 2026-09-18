use crate::cast::CastExpr;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum CastParseError {
    #[error("unexpected end of input at byte {pos}")]
    UnexpectedEof { pos: usize },
    #[error("unexpected byte {found:?} at position {pos}")]
    UnexpectedByte { pos: usize, found: u8 },
    #[error("expected {expected:?} at position {pos}")]
    Expected { pos: usize, expected: u8 },
    #[error("unknown cast identifier `{ident}`")]
    UnknownIdentifier { ident: String },
    #[error("invalid integer argument `{raw}`")]
    InvalidInteger { raw: String },
    #[error("trailing input starting at byte {pos}")]
    TrailingInput { pos: usize },
}

pub fn parse(data: &[u8]) -> Result<CastExpr, CastParseError> {
    let mut parser = Parser::new(data);
    let expr = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.input.len() {
        return Err(CastParseError::TrailingInput { pos: parser.pos });
    }
    Ok(expr)
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            input: data,
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Result<u8, CastParseError> {
        self.skip_ws();
        self.input
            .get(self.pos)
            .copied()
            .ok_or(CastParseError::UnexpectedEof { pos: self.pos })
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn expect(&mut self, expected: u8) -> Result<(), CastParseError> {
        let found = self.peek()?;
        if found != expected {
            return Err(CastParseError::Expected {
                pos: self.pos,
                expected,
            });
        }
        self.bump();
        Ok(())
    }

    fn parse_value(&mut self) -> Result<CastExpr, CastParseError> {
        match self.peek()? {
            b'[' => self.parse_array(),
            b'a'..=b'z' | b'A'..=b'Z' => self.parse_ident(),
            found => Err(CastParseError::UnexpectedByte {
                pos: self.pos,
                found,
            }),
        }
    }

    fn parse_int_arg(&mut self) -> Result<usize, CastParseError> {
        self.expect(b'(')?;
        self.skip_ws();
        let start = self.pos;

        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }

        let raw = std::str::from_utf8(&self.input[start..self.pos]).unwrap_or("");
        let arg = raw
            .parse::<usize>()
            .map_err(|_| CastParseError::InvalidInteger {
                raw: raw.to_owned(),
            })?;

        self.expect(b')')?;
        Ok(arg)
    }

    fn parse_ident(&mut self) -> Result<CastExpr, CastParseError> {
        let start = self.pos;

        while self.pos < self.input.len()
            && (self.input[self.pos].is_ascii_alphabetic() || self.input[self.pos].is_ascii_digit())
        {
            self.pos += 1;
        }

        let ident = std::str::from_utf8(&self.input[start..self.pos]).map_err(|_| {
            CastParseError::UnexpectedByte {
                pos: start,
                found: self.input[start],
            }
        })?;

        match ident {
            "String" => Ok(CastExpr::String),
            "Int" => Ok(CastExpr::Int),
            "Float" => Ok(CastExpr::Float),
            "Bool" => Ok(CastExpr::Bool),
            "Nullable" => Ok(CastExpr::Nullable(Box::new(self.parse_paren_expr()?))),
            "DecStrToInt" => Ok(CastExpr::DecStrToInt(self.parse_int_arg()?)),
            _ => Err(CastParseError::UnknownIdentifier {
                ident: ident.to_owned(),
            }),
        }
    }

    fn parse_paren_expr(&mut self) -> Result<CastExpr, CastParseError> {
        self.expect(b'(')?;
        let expr = self.parse_value()?;
        self.expect(b')')?;
        Ok(expr)
    }

    fn parse_array(&mut self) -> Result<CastExpr, CastParseError> {
        self.expect(b'[')?;

        let mut values = Vec::new();

        while self.peek()? != b']' {
            values.push(self.parse_value()?);

            if self.peek()? == b',' {
                self.bump();
            }
        }

        self.expect(b']')?;
        Ok(CastExpr::Tuple(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_expressions() {
        assert_eq!(parse(b"String").unwrap(), CastExpr::String);
        assert_eq!(parse(b"Int").unwrap(), CastExpr::Int);
        assert_eq!(parse(b"Float").unwrap(), CastExpr::Float);
        assert_eq!(parse(b"Bool").unwrap(), CastExpr::Bool);
        assert_eq!(
            parse(b"Nullable(Float)").unwrap(),
            CastExpr::Nullable(Box::new(CastExpr::Float))
        );
    }

    #[test]
    fn array_expressions() {
        let out = parse(b"[String, DecStrToInt(10)]").unwrap();
        assert_eq!(
            out,
            CastExpr::Tuple(vec![CastExpr::String, CastExpr::DecStrToInt(10)])
        );
    }

    #[test]
    fn complex_array_expressions() {
        let out = parse(b"[String,String,[String,Int]]").unwrap();
        assert_eq!(
            out,
            CastExpr::Tuple(vec![
                CastExpr::String,
                CastExpr::String,
                CastExpr::Tuple(vec![CastExpr::String, CastExpr::Int])
            ])
        );
    }

    #[test]
    fn unknown_identifier() {
        assert_eq!(
            parse(b"Nope"),
            Err(CastParseError::UnknownIdentifier {
                ident: "Nope".into()
            })
        );
    }

    #[test]
    fn missing_paren() {
        assert_eq!(
            parse(b"DecStrToInt)"),
            Err(CastParseError::Expected {
                pos: 11,
                expected: b'('
            })
        );
    }
}
