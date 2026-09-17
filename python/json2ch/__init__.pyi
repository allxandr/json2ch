from collections.abc import Sequence

class Column:
    name: str
    type: str
    path: str
    cast: str

    def __init__(self, name: str, type: str, path: str, cast: str) -> None: ...
    def __repr__(self) -> str: ...

_ColumnSpec = Column | tuple[str, str, str, str]

class Encoder:
    def __init__(self, columns: Sequence[_ColumnSpec]) -> None: ...
    def encode(self, json: bytes) -> bytes: ...

def encode(json: bytes, columns: Sequence[_ColumnSpec]) -> bytes: ...
