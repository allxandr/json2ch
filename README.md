# json2ch

Minimal Python library (Rust via [maturin](https://www.maturin.rs/)) that turns JSON bytes into ClickHouse [RowBinary](https://clickhouse.com/docs/interfaces/formats/RowBinary).


## Install

```bash
pip install maturin
maturin develop
```

## Usage

Each column is a `Column` (or a `(name, clickhouse_type, jsonpath, cast)` tuple):

```python
import json2ch

row = json2ch.encode(
    b'{"symbol":"BTCUSDT","asks":[["1.50","0.01"]]}',
    [
        json2ch.Column("symbol", "String", "$.symbol", "String"),
        json2ch.Column(
            "asks",
            "Array(Tuple(Int64, Int64))",
            "$.asks[*]",
            "[DecStrToInt(2), DecStrToInt(5)]",
        ),
    ],
)
```

Reuse a compiled encoder for many documents:

```python
enc = json2ch.Encoder([
    json2ch.Column(name="symbol", type="String", path="$.symbol", cast="String"),
    json2ch.Column(
        name="asks",
        type="Array(Tuple(Int64, Int64))",
        path="$.asks[*]",
        cast="[DecStrToInt(2), DecStrToInt(5)]",
    ),
])
payload = enc.encode(json_bytes)
```

Insert with:

```sql
INSERT INTO t (symbol, asks) FORMAT RowBinary
```

## Casts

| Cast | JSON input | Result |
| --- | --- | --- |
| `String` | string | `String` |
| `Int` | number or integer string | `Int64` |
| `DecStrToInt(n)` | decimal string | `Int64` ticks (`value * 10^n`) |
| `[T1, T2, ...]` | array | `Tuple` |

`Array(...)` columns collect JSONPath matches. Point the path at the elements (`$.asks[*]`, `$.tags[*]`), not the array itself.
