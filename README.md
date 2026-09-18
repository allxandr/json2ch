# json2ch

Minimal Python library (Rust via [maturin](https://www.maturin.rs/)) that turns JSON bytes into ClickHouse [RowBinary](https://clickhouse.com/docs/interfaces/formats/RowBinary).

JSONPath extraction, type casts, and RowBinary encoding all run in Rust. Python only hands in bytes and gets bytes back — nothing is materialized as intermediate Python objects, which is why this stays faster than parse-in-Python-then-encode pipelines.

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
| `Int` | number, whole float, or integer string | `Int64` |
| `Float` | number or numeric string | `Float64` (or `Float32` if the column type is `Float32`) |
| `Bool` | `true`/`false`, `0`/`1`, or `"true"`/`"false"` | `Bool` |
| `Nullable(T)` | JSON `null` or a value of `T` | `Nullable(...)` |
| `DecStrToInt(n)` | decimal string | `Int64` ticks (`value * 10^n`) |
| `[T1, T2, ...]` | array | `Tuple` |

`Array(...)` columns collect JSONPath matches. Point the path at the elements (`$.asks[*]`, `$.tags[*]`), not the array itself.

## Benchmark

2500 documents simulating exchange orderbook response, each with 200 used ask + bid levels **and** a large unused `ignored` subtree (1200 nested events + 32 KiB of notes). Input is 460 MB; only `symbol`, `seq`, `asks`, `bids`, and `tags` are encoded.

Python version uses `json` / `orjson` and then writes RowBinary with [`clickhouse-rowbinary`](https://pypi.org/project/clickhouse-rowbinary/), which is already fast rust-backed lib. json2ch version uses its encoder directly. Outputs were checked equal.

Measured with `maturin develop --release` on Python 3.14.6, macOS arm64, median of 5 rounds:

| Pipeline | Time | docs/s | MB/s | relative |
| --- | ---: | ---: | ---: | ---: |
| **json2ch** | 0.984s | 2,541 | 467 | 1.00× |
| json + clickhouse-rowbinary | 3.954s | 632 | 116 | 4.02× |
| orjson + clickhouse-rowbinary | 2.471s | 1,012 | 186 | 2.51× |

Reproduce:

```bash
pip install -r benches/requirements.txt
maturin develop --release
python benches/bench.py
```

## Credits

- [rsonpath](https://github.com/V0ldek/rsonpath) — JSONPath execution
- [clickhouse-rowbinary](https://github.com/dovreshef/clickhouse-rowbinary) — ClickHouse RowBinary encoding
