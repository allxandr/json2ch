use crate::cast::{decimal_str_to_ticks, CastError, CastExpr, Cell};
use serde_json::value::RawValue;

#[derive(Debug, Clone, PartialEq)]
pub enum Applied {
    Scalar(Cell),
    Null,
    Tuple(Vec<Applied>),
}

/// Parse a JSON fragment according to `expr`.
///
/// The first non-whitespace byte is checked against the expected kind so a
/// large object/array is rejected before `serde_json` walks it.
pub fn apply_json(expr: &CastExpr, bytes: &[u8]) -> Result<Applied, CastError> {
    match expr {
        CastExpr::Nullable(inner) => {
            if is_json_null(bytes)? {
                Ok(Applied::Null)
            } else {
                apply_json(inner, bytes)
            }
        }
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
                Ok(Applied::Scalar(Cell::Int(json_i64(bytes)?)))
            }
            _ => Err(CastError::Apply("expected JSON integer".into())),
        },
        CastExpr::Float => match first_non_ws(bytes) {
            Some(b'"') => {
                let s: String = serde_json::from_slice(bytes)?;
                let f = s
                    .parse::<f64>()
                    .map_err(|_| CastError::Apply(format!("invalid float `{s}`")))?;
                Ok(Applied::Scalar(Cell::Float(f)))
            }
            Some(b) if b == b'-' || b.is_ascii_digit() => {
                Ok(Applied::Scalar(Cell::Float(serde_json::from_slice(bytes)?)))
            }
            _ => Err(CastError::Apply("expected JSON number".into())),
        },
        CastExpr::Bool => Ok(Applied::Scalar(Cell::Bool(json_bool(bytes)?))),
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

fn is_json_null(bytes: &[u8]) -> Result<bool, CastError> {
    if first_non_ws(bytes) != Some(b'n') {
        return Ok(false);
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    Ok(value.is_null())
}

fn parse_i64(s: &str) -> Result<i64, CastError> {
    s.parse::<i64>()
        .map_err(|_| CastError::Apply(format!("invalid integer `{s}`")))
}

fn json_i64(bytes: &[u8]) -> Result<i64, CastError> {
    if let Ok(n) = serde_json::from_slice::<i64>(bytes) {
        return Ok(n);
    }
    let f: f64 = serde_json::from_slice(bytes)
        .map_err(|_| CastError::Apply("expected JSON integer".into()))?;
    if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
        Ok(f as i64)
    } else {
        Err(CastError::Apply("expected JSON integer".into()))
    }
}

fn json_bool(bytes: &[u8]) -> Result<bool, CastError> {
    match first_non_ws(bytes) {
        Some(b't' | b'f') => Ok(serde_json::from_slice(bytes)?),
        Some(b'0' | b'1') => match json_i64(bytes)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(CastError::Apply("expected JSON bool".into())),
        },
        Some(b'"') => {
            let s: String = serde_json::from_slice(bytes)?;
            match s.to_ascii_lowercase().as_str() {
                "true" | "1" => Ok(true),
                "false" | "0" => Ok(false),
                _ => Err(CastError::Apply(format!("invalid bool `{s}`"))),
            }
        }
        _ => Err(CastError::Apply("expected JSON bool".into())),
    }
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
    fn applies_int_from_string_and_whole_float() {
        assert_eq!(
            apply_json(&CastExpr::Int, br#""7""#).unwrap(),
            Applied::Scalar(Cell::Int(7))
        );
        assert_eq!(
            apply_json(&CastExpr::Int, b"1.0").unwrap(),
            Applied::Scalar(Cell::Int(1))
        );
    }

    #[test]
    fn applies_float_from_number_and_string() {
        assert_eq!(
            apply_json(&CastExpr::Float, b"1.5").unwrap(),
            Applied::Scalar(Cell::Float(1.5))
        );
        assert_eq!(
            apply_json(&CastExpr::Float, br#""2.25""#).unwrap(),
            Applied::Scalar(Cell::Float(2.25))
        );
        assert_eq!(
            apply_json(&CastExpr::Float, b"3").unwrap(),
            Applied::Scalar(Cell::Float(3.0))
        );
    }

    #[test]
    fn applies_bool_from_json_and_string() {
        assert_eq!(
            apply_json(&CastExpr::Bool, b"true").unwrap(),
            Applied::Scalar(Cell::Bool(true))
        );
        assert_eq!(
            apply_json(&CastExpr::Bool, br#""false""#).unwrap(),
            Applied::Scalar(Cell::Bool(false))
        );
        assert_eq!(
            apply_json(&CastExpr::Bool, b"1").unwrap(),
            Applied::Scalar(Cell::Bool(true))
        );
    }

    #[test]
    fn applies_nullable_null_and_value() {
        let expr = CastExpr::Nullable(Box::new(CastExpr::Int));
        assert_eq!(apply_json(&expr, b"null").unwrap(), Applied::Null);
        assert_eq!(
            apply_json(&expr, b"4").unwrap(),
            Applied::Scalar(Cell::Int(4))
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
