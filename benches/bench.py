"""Benchmark json2ch vs json/orjson + clickhouse-rowbinary.

Python must fully parse each document (including a large unused subtree).
json2ch only JSONPath-extracts the columns it needs.
"""

from __future__ import annotations

import argparse
import gc
import json
import platform
import statistics
import sys
import time

import orjson
from clickhouse_rowbinary import RowBinaryWriter, Schema

import json2ch

PRICE_SCALE = 2
QTY_SCALE = 5


def dec_str_to_int(s: str, scale: int) -> int:
    s = s.strip()
    if "." in s:
        whole, frac = s.split(".", 1)
    else:
        whole, frac = s, ""
    if len(frac) > scale:
        frac = frac[:scale]
    frac_ticks = int(frac) if frac else 0
    frac_ticks *= 10 ** (scale - len(frac))
    whole_ticks = int(whole) if whole else 0
    return whole_ticks * (10**scale) + frac_ticks


def unused_subtree(n_events: int, padding_kb: int) -> dict:
    events = [
        {
            "id": i,
            "side": "buy" if i % 2 == 0 else "sell",
            "price": f"{100 + (i % 900)}.{i % 100:02d}",
            "qty": f"0.{i % 100000:05d}",
            "user": {
                "id": i % 10_000,
                "label": f"user-{i % 10_000}",
                "flags": ["a", "b", "c"],
            },
        }
        for i in range(n_events)
    ]
    return {
        "events": events,
        "notes": "x" * (padding_kb * 1024),
        "stats": {f"m{k}": k * 1.5 for k in range(200)},
    }


def make_payloads(
    n_docs: int, n_levels: int, n_unused: int, padding_kb: int
) -> list[bytes]:
    ignored = unused_subtree(n_unused, padding_kb)
    payloads: list[bytes] = []
    for i in range(n_docs):
        asks = [
            [f"{1000 + j}.{j % 100:02d}", f"0.{(j * 17) % 100000:05d}"]
            for j in range(n_levels)
        ]
        bids = [
            [f"{999 - (j % 500)}.{j % 100:02d}", f"0.{(j * 13) % 100000:05d}"]
            for j in range(n_levels)
        ]
        payloads.append(
            orjson.dumps(
                {
                    "symbol": f"SYM{i % 50:02d}USDT",
                    "seq": i,
                    "asks": asks,
                    "bids": bids,
                    "tags": ["spot", "usdt", f"b{i % 8}"],
                    "ignored": ignored,
                }
            )
        )
    return payloads


def extract_row(obj: dict) -> dict:
    return {
        "symbol": obj["symbol"],
        "seq": obj["seq"],
        "asks": [
            (dec_str_to_int(p, PRICE_SCALE), dec_str_to_int(q, QTY_SCALE))
            for p, q in obj["asks"]
        ],
        "bids": [
            (dec_str_to_int(p, PRICE_SCALE), dec_str_to_int(q, QTY_SCALE))
            for p, q in obj["bids"]
        ],
        "tags": obj["tags"],
    }


def python_pipeline(payloads: list[bytes], loads) -> bytes:
    schema = Schema.from_clickhouse(
        [
            ("symbol", "String"),
            ("seq", "Int64"),
            ("asks", "Array(Tuple(Int64, Int64))"),
            ("bids", "Array(Tuple(Int64, Int64))"),
            ("tags", "Array(String)"),
        ]
    )
    writer = RowBinaryWriter(schema)
    for raw in payloads:
        writer.write_row(extract_row(loads(raw)))
    return writer.take()


def json2ch_pipeline(payloads: list[bytes], enc: json2ch.Encoder) -> bytes:
    out = bytearray()
    for raw in payloads:
        out += enc.encode(raw)
    return bytes(out)


def json2ch_encoder() -> json2ch.Encoder:
    return json2ch.Encoder(
        [
            json2ch.Column("symbol", "String", "$.symbol", "String"),
            json2ch.Column("seq", "Int64", "$.seq", "Int"),
            json2ch.Column(
                "asks",
                "Array(Tuple(Int64, Int64))",
                "$.asks[*]",
                f"[DecStrToInt({PRICE_SCALE}), DecStrToInt({QTY_SCALE})]",
            ),
            json2ch.Column(
                "bids",
                "Array(Tuple(Int64, Int64))",
                "$.bids[*]",
                f"[DecStrToInt({PRICE_SCALE}), DecStrToInt({QTY_SCALE})]",
            ),
            json2ch.Column("tags", "Array(String)", "$.tags[*]", "String"),
        ]
    )


def timed(fn, rounds: int) -> list[float]:
    times = []
    for _ in range(rounds):
        gc.collect()
        t0 = time.perf_counter()
        fn()
        times.append(time.perf_counter() - t0)
    return times


def fmt_num(n: float) -> str:
    if n >= 1000:
        return f"{n:,.0f}"
    if n >= 100:
        return f"{n:.1f}"
    return f"{n:.2f}"


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--docs", type=int, default=2500)
    p.add_argument("--levels", type=int, default=200)
    p.add_argument("--unused-events", type=int, default=1200)
    p.add_argument("--padding-kb", type=int, default=32)
    p.add_argument("--rounds", type=int, default=5)
    args = p.parse_args()

    print("generating payloads...", flush=True)
    payloads = make_payloads(
        args.docs, args.levels, args.unused_events, args.padding_kb
    )
    input_bytes = sum(map(len, payloads))
    enc = json2ch_encoder()

    print("checking output equality...", flush=True)
    rust_out = json2ch_pipeline(payloads[:3], enc)
    py_out = python_pipeline(payloads[:3], orjson.loads)
    if rust_out != py_out:
        raise SystemExit(
            f"output mismatch: json2ch {len(rust_out)}B vs python {len(py_out)}B"
        )

    jobs = [
        ("json2ch", lambda: json2ch_pipeline(payloads, enc)),
        ("json + clickhouse-rowbinary", lambda: python_pipeline(payloads, json.loads)),
        (
            "orjson + clickhouse-rowbinary",
            lambda: python_pipeline(payloads, orjson.loads),
        ),
    ]

    print("warmup...", flush=True)
    for _, fn in jobs:
        fn()

    results = []
    for name, fn in jobs:
        print(f"bench {name}...", flush=True)
        times = timed(fn, args.rounds)
        results.append((name, statistics.median(times)))

    base = results[0][1]
    print()
    print(
        f"Python {sys.version.split()[0]}  {platform.platform()}  "
        f"{platform.processor() or platform.machine()}"
    )
    print(
        f"{args.docs} docs, {args.levels} used ask+bid levels, "
        f"{args.unused_events} unused events + {args.padding_kb} KiB notes/doc, "
        f"{input_bytes / 1e6:.1f} MB input, median of {args.rounds} rounds"
    )
    print()
    print("| Pipeline | Time | docs/s | MB/s | relative |")
    print("| --- | ---: | ---: | ---: | ---: |")
    for name, med in results:
        docs_s = args.docs / med
        mb_s = (input_bytes / 1e6) / med
        rel = med / base
        print(
            f"| {name} | {med:.3f}s | {fmt_num(docs_s)} | {fmt_num(mb_s)} | {rel:.2f}× |"
        )


if __name__ == "__main__":
    main()
