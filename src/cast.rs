mod apply;
mod decimal;
mod parser;

pub use apply::{apply_json, Applied};
pub use decimal::{decimal_str_to_ticks, TickParseError};
pub use parser::{parse, CastParseError};

use thiserror::Error;

#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CastExpr {
    Tuple(Vec<CastExpr>),
    Nullable(Box<CastExpr>),
    String,
    Int,
    Float,
    Bool,
    DecStrToInt(usize),
}

#[derive(Error, Debug)]
pub enum CastError {
    #[error(transparent)]
    Parse(#[from] CastParseError),
    #[error(transparent)]
    Tick(#[from] TickParseError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Apply(String),
}
