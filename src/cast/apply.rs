use crate::cast::{decimal_str_to_ticks, CastError, CastExpr, Cell};
use serde_json::value::RawValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    Scalar(Cell),
    Tuple(Vec<Applied>),
}

/// Parse a JSON fragment according to `expr`.
///
/// The first non-whitespace byte is checked against the expected kind so a
/// large object/array is rejected before `serde_json` walks it.
pub fn apply_json(expr: &CastExpr, bytes: &[u8]) -> Result<Applied, CastError> {
    match expr {
        CastExpr::String => {
            expect_start(bytes, |b| b == b'"', "expected JSON string")?;
            Ok(Applied::Scalar(Cell::String(serde_json::from_slice(
                bytes,
            )?)))
        }
        CastExpr::Int => match first_non_ws(bytes) {
            Some(b'"') => {
                let s: String = serde_json::from_slice(bytes)?;
                Ok(Applied::Scalar(Cell::Int(parse_i64(&s)?)))
            }
            Some(b) if b == b'-' || b.is_ascii_digit() => {
                Ok(Applied::Scalar(Cell::Int(serde_json::from_slice(bytes)?)))
            }
            _ => Err(CastError::Apply("expected JSON integer".into())),
        },
        CastExpr::DecStrToInt(scale) => {
            expect_start(bytes, |b| b == b'"', "expected JSON string")?;
            let s: String = serde_json::from_slice(bytes)?;
            let scale = u32::try_from(*scale)
                .map_err(|_| CastError::Apply(format!("scale {scale} does not fit in u32")))?;
            Ok(Applied::Scalar(Cell::Int(decimal_str_to_ticks(&s, scale)?)))
        }
        CastExpr::Tuple(items) => {
            expect_start(bytes, |b| b == b'[', "expected JSON array")?;
            let children: Vec<Box<RawValue>> = serde_json::from_slice(bytes)?;
            if children.len() != items.len() {
                return Err(CastError::Apply(format!(
                    "expected {} tuple elements, got {}",
                    items.len(),
                    children.len()
                )));
            }
            items
                .iter()
                .zip(children.iter())
                .map(|(item, child)| apply_json(item, child.get().as_bytes()))
                .collect::<Result<Vec<_>, _>>()
                .map(Applied::Tuple)
        }
    }
}

fn first_non_ws(bytes: &[u8]) -> Option<u8> {
    bytes.iter().copied().find(|b| !b.is_ascii_whitespace())
}

fn expect_start(bytes: &[u8], ok: impl Fn(u8) -> bool, msg: &'static str) -> Result<(), CastError> {
    match first_non_ws(bytes) {
        Some(b) if ok(b) => Ok(()),
        _ => Err(CastError::Apply(msg.into())),
    }
}

fn parse_i64(s: &str) -> Result<i64, CastError> {
    s.parse::<i64>()
        .map_err(|_| CastError::Apply(format!("invalid integer `{s}`")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cast::CastExpr;

    #[test]
    fn applies_dec_str_to_json_string() {
        let expr = CastExpr::DecStrToInt(2);
        assert_eq!(
            apply_json(&expr, br#""1.5""#).unwrap(),
            Applied::Scalar(Cell::Int(150))
        );
    }

    #[test]
    fn applies_string() {
        let expr = CastExpr::String;
        assert_eq!(
            apply_json(&expr, br#""hello""#).unwrap(),
            Applied::Scalar(Cell::String("hello".into()))
        );
    }

    #[test]
    fn applies_int_to_json_number() {
        let expr = CastExpr::Int;
        assert_eq!(
            apply_json(&expr, b"42").unwrap(),
            Applied::Scalar(Cell::Int(42))
        );
    }

    #[test]
    fn applies_tuple_to_json_array() {
        let expr = CastExpr::Tuple(vec![CastExpr::String, CastExpr::DecStrToInt(2)]);
        assert_eq!(
            apply_json(&expr, br#"["1.50","2.5"]"#).unwrap(),
            Applied::Tuple(vec![
                Applied::Scalar(Cell::String("1.50".into())),
                Applied::Scalar(Cell::Int(250)),
            ])
        );
    }

    #[test]
    fn tuple_arity_mismatch_is_error() {
        let expr = CastExpr::Tuple(vec![CastExpr::String, CastExpr::Int]);
        assert!(apply_json(&expr, br#"["only-one"]"#).is_err());
    }

    #[test]
    fn string_rejects_object_on_first_byte() {
        let err = apply_json(
            &CastExpr::String,
            br#"{"this":"is","a":"big","object":true}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("expected JSON string"));
    }
}
