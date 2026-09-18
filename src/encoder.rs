use crate::cast::{apply_json, Applied, CastError, CastExpr, Cell};
use clickhouse_rowbinary::{RowBinaryFormat, RowBinaryValueWriter, Schema, Value};
use rsonpath::engine::{Compiler, Engine, RsonpathEngine};
use rsonpath::input::BorrowedBytes;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Cast(#[from] CastError),
    #[error(transparent)]
    CastParse(#[from] crate::cast::CastParseError),
    #[error("jsonpath: {0}")]
    JsonPath(String),
    #[error("clickhouse rowbinary: {0}")]
    RowBinary(#[from] clickhouse_rowbinary::Error),
    #[error("{0}")]
    Message(String),
}

pub struct Column {
    pub name: String,
    pub ch_type: String,
    pub path: String,
    pub cast: String,
}

struct CompiledColumn {
    engine: RsonpathEngine,
    cast: CastExpr,
    is_array: bool,
    is_nullable: bool,
    ch_type: String,
}

pub struct Encoder {
    columns: Vec<CompiledColumn>,
    schema: Schema,
}

impl Encoder {
    pub fn new(columns: Vec<Column>) -> Result<Self, Error> {
        let pairs: Vec<(&str, &str)> = columns
            .iter()
            .map(|col| (col.name.as_str(), col.ch_type.as_str()))
            .collect();
        let schema = Schema::from_type_strings(&pairs)?;

        let compiled = columns
            .into_iter()
            .map(|col| {
                Ok(CompiledColumn {
                    is_array: is_array_type(&col.ch_type),
                    is_nullable: is_nullable_type(&col.ch_type),
                    engine: compile_path(&col.path)?,
                    cast: crate::cast::parse(col.cast.as_bytes())?,
                    ch_type: col.ch_type,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;

        Ok(Self {
            columns: compiled,
            schema,
        })
    }

    pub fn encode(&self, json: &[u8]) -> Result<Vec<u8>, Error> {
        let mut row = Vec::with_capacity(self.columns.len());
        for col in &self.columns {
            row.push(column_value(json, col)?);
        }

        let mut writer =
            RowBinaryValueWriter::new(Vec::new(), RowBinaryFormat::RowBinary, self.schema.clone());
        writer.write_header()?;
        writer.write_row(&row)?;
        Ok(writer.into_inner())
    }
}

fn is_array_type(ch_type: &str) -> bool {
    ch_type.trim().starts_with("Array(")
}

fn is_nullable_type(ch_type: &str) -> bool {
    ch_type.trim().starts_with("Nullable(")
}

fn type_core(ch_type: &str) -> String {
    let mut t = ch_type.trim();
    loop {
        t = t.trim();
        if let Some(inner) = t.strip_prefix("Array(").and_then(|s| s.strip_suffix(')')) {
            t = inner;
            continue;
        }
        if let Some(inner) = t
            .strip_prefix("Nullable(")
            .and_then(|s| s.strip_suffix(')'))
        {
            t = inner;
            continue;
        }
        return t.replace(' ', "");
    }
}

fn compile_path(path: &str) -> Result<RsonpathEngine, Error> {
    let query = rsonpath_syntax::parse(path).map_err(|err| Error::JsonPath(err.to_string()))?;
    RsonpathEngine::compile_query(&query).map_err(|err| Error::JsonPath(err.to_string()))
}

fn column_value(json: &[u8], col: &CompiledColumn) -> Result<Value, Error> {
    let matches = extract_matches(json, &col.engine)?;
    if col.is_array {
        let values = matches
            .iter()
            .map(|value| applied_to_value(&col.cast, apply_json(&col.cast, value)?, &col.ch_type))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(values))
    } else {
        match matches.as_slice() {
            [value] => applied_to_value(&col.cast, apply_json(&col.cast, value)?, &col.ch_type),
            [] if col.is_nullable => Ok(Value::Nullable(None)),
            [] => Err(Error::Message("JSONPath matched no values".into())),
            _ => Err(Error::Message(
                "JSONPath matched multiple values for a non-Array column".into(),
            )),
        }
    }
}

fn extract_matches(body: &[u8], engine: &RsonpathEngine) -> Result<Vec<Vec<u8>>, Error> {
    let input = BorrowedBytes::new(body);
    let mut matches = Vec::new();
    engine
        .matches(&input, &mut matches)
        .map_err(|err| Error::JsonPath(err.to_string()))?;
    Ok(matches.into_iter().map(|m| m.into_bytes()).collect())
}

fn applied_to_value(expr: &CastExpr, applied: Applied, ch_type: &str) -> Result<Value, Error> {
    match expr {
        CastExpr::Nullable(inner) => match applied {
            Applied::Null => Ok(Value::Nullable(None)),
            other => Ok(Value::Nullable(Some(Box::new(applied_to_value(
                inner, other, ch_type,
            )?)))),
        },
        CastExpr::Tuple(items) => {
            let Applied::Tuple(parts) = applied else {
                return Err(Error::Message("expected a tuple value".into()));
            };
            items
                .iter()
                .zip(parts)
                .map(|(item, part)| applied_to_value(item, part, ch_type))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Tuple)
        }
        CastExpr::String => match applied {
            Applied::Scalar(Cell::String(s)) => Ok(Value::String(s.into_bytes())),
            _ => Err(Error::Message("expected a string value".into())),
        },
        CastExpr::Int | CastExpr::DecStrToInt(_) => match applied {
            Applied::Scalar(Cell::Int(n)) => Ok(Value::Int64(n)),
            _ => Err(Error::Message("expected an int value".into())),
        },
        CastExpr::Float => match applied {
            Applied::Scalar(Cell::Float(f)) => Ok(encode_float(f, ch_type)),
            _ => Err(Error::Message("expected a float value".into())),
        },
        CastExpr::Bool => match applied {
            Applied::Scalar(Cell::Bool(b)) => Ok(Value::Bool(b)),
            _ => Err(Error::Message("expected a bool value".into())),
        },
    }
}

fn encode_float(f: f64, ch_type: &str) -> Value {
    if type_core(ch_type) == "Float32" {
        Value::Float32(f as f32)
    } else {
        Value::Float64(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoder(columns: &[(&str, &str, &str, &str)]) -> Encoder {
        Encoder::new(
            columns
                .iter()
                .map(|(name, ch_type, path, cast)| Column {
                    name: (*name).into(),
                    ch_type: (*ch_type).into(),
                    path: (*path).into(),
                    cast: (*cast).into(),
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn encodes_scalars() {
        let enc = encoder(&[
            ("name", "String", "$.name", "String"),
            ("n", "Int64", "$.n", "Int"),
        ]);
        let out = enc.encode(br#"{"name":"btc","n":7}"#).unwrap();
        assert!(!out.is_empty());
    }

    #[test]
    fn encodes_array_of_tuples_from_star_path() {
        let enc = encoder(&[(
            "asks",
            "Array(Tuple(Int64, Int64))",
            "$.asks[*]",
            "[DecStrToInt(2), DecStrToInt(5)]",
        )]);
        let json = br#"{"asks":[["1.50","0.01"],["2.00","0.02"]]}"#;
        let out = enc.encode(json).unwrap();
        assert!(!out.is_empty());
    }

    #[test]
    fn encodes_single_tuple_match_as_one_array_element() {
        let enc = encoder(&[(
            "asks",
            "Array(Tuple(Int64, Int64))",
            "$.asks[*]",
            "[DecStrToInt(2), DecStrToInt(5)]",
        )]);
        enc.encode(br#"{"asks":[["1.50","0.01"]]}"#).unwrap();
    }

    #[test]
    fn encodes_array_of_strings() {
        let enc = encoder(&[("tags", "Array(String)", "$.tags[*]", "String")]);
        let out = enc.encode(br#"{"tags":["a","b"]}"#).unwrap();
        assert!(!out.is_empty());
    }

    #[test]
    fn empty_array_is_ok() {
        let enc = encoder(&[("tags", "Array(String)", "$.tags[*]", "String")]);
        enc.encode(br#"{"tags":[]}"#).unwrap();
    }

    #[test]
    fn roundtrip_orderbook_like_json() {
        let enc = encoder(&[
            ("symbol", "String", "$.symbol", "String"),
            (
                "asks",
                "Array(Tuple(Int64, Int64))",
                "$.asks[*]",
                "[DecStrToInt(2), DecStrToInt(5)]",
            ),
        ]);
        let json = br#"{"symbol":"BTCUSDT","asks":[["1.50","0.01"],["2.00","0.02"]]}"#;
        let bytes = enc.encode(json).unwrap();
        let schema = Schema::from_type_strings(&[
            ("symbol", "String"),
            ("asks", "Array(Tuple(Int64, Int64))"),
        ])
        .unwrap();
        let mut reader = clickhouse_rowbinary::RowBinaryValueReader::with_schema(
            bytes.as_slice(),
            RowBinaryFormat::RowBinary,
            schema,
        )
        .unwrap();
        let row = reader.read_row().unwrap().unwrap();
        assert_eq!(row[0], Value::String(b"BTCUSDT".to_vec()));
        assert_eq!(
            row[1],
            Value::Array(vec![
                Value::Tuple(vec![Value::Int64(150), Value::Int64(1000)]),
                Value::Tuple(vec![Value::Int64(200), Value::Int64(2000)]),
            ])
        );
    }

    #[test]
    fn encodes_float_bool_and_null() {
        let enc = encoder(&[
            ("px", "Float64", "$.px", "Float"),
            ("ok", "Bool", "$.ok", "Bool"),
            ("note", "Nullable(String)", "$.note", "Nullable(String)"),
        ]);
        let bytes = enc
            .encode(br#"{"px":"1.5","ok":true,"note":null}"#)
            .unwrap();
        let schema = Schema::from_type_strings(&[
            ("px", "Float64"),
            ("ok", "Bool"),
            ("note", "Nullable(String)"),
        ])
        .unwrap();
        let mut reader = clickhouse_rowbinary::RowBinaryValueReader::with_schema(
            bytes.as_slice(),
            RowBinaryFormat::RowBinary,
            schema,
        )
        .unwrap();
        let row = reader.read_row().unwrap().unwrap();
        assert_eq!(row[0], Value::Float64(1.5));
        assert_eq!(row[1], Value::Bool(true));
        assert_eq!(row[2], Value::Nullable(None));
    }

    #[test]
    fn nullable_missing_path_is_null() {
        let enc = encoder(&[("note", "Nullable(Int64)", "$.note", "Nullable(Int)")]);
        let bytes = enc.encode(br#"{"other":1}"#).unwrap();
        let schema = Schema::from_type_strings(&[("note", "Nullable(Int64)")]).unwrap();
        let mut reader = clickhouse_rowbinary::RowBinaryValueReader::with_schema(
            bytes.as_slice(),
            RowBinaryFormat::RowBinary,
            schema,
        )
        .unwrap();
        let row = reader.read_row().unwrap().unwrap();
        assert_eq!(row[0], Value::Nullable(None));
    }
}
