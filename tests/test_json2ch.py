import json2ch


def test_encode_scalars_and_array_of_tuples():
    payload = json2ch.encode(
        b'{"symbol":"BTCUSDT","asks":[["1.50","0.01"],["2.00","0.02"]]}',
        [
            ("symbol", "String", "$.symbol", "String"),
            (
                "asks",
                "Array(Tuple(Int64, Int64))",
                "$.asks[*]",
                "[DecStrToInt(2), DecStrToInt(5)]",
            ),
        ],
    )
    assert isinstance(payload, bytes)
    assert len(payload) > 0


def test_single_tuple_in_array():
    payload = json2ch.encode(
        b'{"asks":[["1.50","0.01"]]}',
        [
            (
                "asks",
                "Array(Tuple(Int64, Int64))",
                "$.asks[*]",
                "[DecStrToInt(2), DecStrToInt(5)]",
            ),
        ],
    )
    assert isinstance(payload, bytes)
    assert len(payload) > 0


def test_encoder_reuse():
    enc = json2ch.Encoder(
        [
            json2ch.Column("n", "Int64", "$.n", "Int"),
            json2ch.Column(name="tags", type="Array(String)", path="$.tags[*]", cast="String"),
        ]
    )
    a = enc.encode(b'{"n":1,"tags":["x"]}')
    b = enc.encode(b'{"n":2,"tags":[]}')
    assert a != b


def test_column_repr_and_attrs():
    col = json2ch.Column("symbol", "String", "$.symbol", "String")
    assert col.name == "symbol"
    assert col.type == "String"
    assert col.path == "$.symbol"
    assert col.cast == "String"
    assert "symbol" in repr(col)
