mod cast;
mod encoder;

use encoder::{Column as RustColumn, Encoder as RustEncoder};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

impl From<encoder::Error> for PyErr {
    fn from(err: encoder::Error) -> PyErr {
        PyValueError::new_err(err.to_string())
    }
}

/// One output column: ClickHouse type, JSONPath, and a cast expression.
#[pyclass(name = "Column")]
#[derive(Clone)]
struct Column {
    #[pyo3(get)]
    name: String,
    #[pyo3(get, name = "type")]
    ch_type: String,
    #[pyo3(get)]
    path: String,
    #[pyo3(get)]
    cast: String,
}

#[pymethods]
impl Column {
    #[new]
    #[pyo3(signature = (name, r#type, path, cast))]
    fn new(name: String, r#type: String, path: String, cast: String) -> Self {
        Self {
            name,
            ch_type: r#type,
            path,
            cast,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Column(name={:?}, type={:?}, path={:?}, cast={:?})",
            self.name, self.ch_type, self.path, self.cast
        )
    }
}

impl From<Column> for RustColumn {
    fn from(col: Column) -> Self {
        Self {
            name: col.name,
            ch_type: col.ch_type,
            path: col.path,
            cast: col.cast,
        }
    }
}

#[derive(FromPyObject)]
enum ColumnArg {
    Column(Column),
    Tuple((String, String, String, String)),
}

impl From<ColumnArg> for RustColumn {
    fn from(arg: ColumnArg) -> Self {
        match arg {
            ColumnArg::Column(col) => col.into(),
            ColumnArg::Tuple((name, ch_type, path, cast)) => Self {
                name,
                ch_type,
                path,
                cast,
            },
        }
    }
}

/// Compiled JSON → ClickHouse RowBinary encoder.
#[pyclass(name = "Encoder")]
struct Encoder {
    inner: RustEncoder,
}

#[pymethods]
impl Encoder {
    #[new]
    fn new(columns: Vec<ColumnArg>) -> PyResult<Self> {
        let columns = columns.into_iter().map(RustColumn::from).collect();
        Ok(Self {
            inner: RustEncoder::new(columns)?,
        })
    }

    /// Encode one JSON document into a single RowBinary row.
    fn encode(&self, json: &[u8]) -> PyResult<Vec<u8>> {
        Ok(self.inner.encode(json)?)
    }
}

/// One-shot helper: compile columns and encode `json` into RowBinary.
#[pyfunction]
fn encode(json: &[u8], columns: Vec<ColumnArg>) -> PyResult<Vec<u8>> {
    Encoder::new(columns)?.encode(json)
}

#[pymodule]
#[pyo3(name = "_json2ch")]
fn json2ch(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Column>()?;
    m.add_class::<Encoder>()?;
    m.add_function(wrap_pyfunction!(encode, m)?)?;
    Ok(())
}
