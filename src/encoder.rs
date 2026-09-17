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
                    engine: compile_path(&col.path)?,
                    cast: crate::cast::parse(col.cast.as_bytes())?,
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

fn compile_path(path: &str) -> Result<RsonpathEngine, Error> {
    let query = rsonpath_syntax::parse(path).map_err(|err| Error::JsonPath(err.to_string()))?;
    RsonpathEngine::compile_query(&query).map_err(|err| Error::JsonPath(err.to_string()))
}

fn column_value(json: &[u8], col: &CompiledColumn) -> Result<Value, Error> {
    let matches = extract_matches(json, &col.engine)?;
    if col.is_array {
        let values = matches
            .iter()
            .map(|value| applied_to_value(apply_json(&col.cast, value)?))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(values))
    } else {
        match matches.as_slice() {
            [value] => Ok(applied_to_value(apply_json(&col.cast, value)?)?),
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

fn applied_to_value(applied: Applied) -> Result<Value, Error> {
    match applied {
        Applied::Scalar(Cell::String(s)) => Ok(Value::String(s.into_bytes())),
        Applied::Scalar(Cell::Int(n)) => Ok(Value::Int64(n)),
        Applied::Tuple(items) => items
            .into_iter()
            .map(applied_to_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Tuple),
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
}
