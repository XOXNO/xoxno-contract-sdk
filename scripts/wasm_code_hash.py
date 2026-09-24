#!/usr/bin/env python3
"""Print the SHA-256 of a WASM module with every custom section removed.

Two builds of one contract that differ only in custom sections (spec docs,
error enums, contract meta) have the same code hash.

Usage: wasm_code_hash.py FILE...
"""
import hashlib
import sys


def read_leb128(data: bytes, pos: int) -> "tuple[int, int]":
    result = shift = 0
    while True:
        byte = data[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if byte < 0x80:
            return result, pos
        shift += 7


def code_hash(data: bytes) -> str:
    if data[:4] != b"\0asm":
        raise ValueError("not a WASM module")
    out = bytearray(data[:8])
    pos = 8
    while pos < len(data):
        start = pos
        section_id = data[pos]
        size, payload = read_leb128(data, pos + 1)
        pos = payload + size
        if section_id != 0:
            out += data[start:pos]
    return hashlib.sha256(bytes(out)).hexdigest()


if __name__ == "__main__":
    for path in sys.argv[1:]:
        with open(path, "rb") as f:
            print(f"{code_hash(f.read())}  {path}")
