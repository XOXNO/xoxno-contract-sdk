#!/usr/bin/env python3
"""Check the embedded WASM files against wasm/MANIFEST.json.

Fails when a file is missing, when its SHA-256 differs from the manifest, or
when a contract's code hash differs from the code hash of the deployed
artifact recorded at sync time.
"""
import hashlib
import json
import pathlib
import sys

from wasm_code_hash import code_hash

WASM_DIR = pathlib.Path(__file__).resolve().parent.parent / "crates" / "xoxno-contract-sdk" / "wasm"


def main() -> int:
    manifest = json.loads((WASM_DIR / "MANIFEST.json").read_text())
    errors = []
    entries = {**manifest["contracts"], **manifest["mocks"]}
    for name, entry in entries.items():
        path = WASM_DIR / entry["file"]
        if not path.is_file():
            errors.append(f"{name}: {entry['file']} is missing")
            continue
        data = path.read_bytes()
        if hashlib.sha256(data).hexdigest() != entry["sha256"]:
            errors.append(f"{name}: sha256 differs from MANIFEST.json")
        if "code_sha256" in entry and code_hash(data) != entry["code_sha256"]:
            errors.append(f"{name}: code hash differs from MANIFEST.json")
    listed = {entry["file"] for entry in entries.values()}
    for path in WASM_DIR.glob("*.wasm"):
        if path.name not in listed:
            errors.append(f"{path.name} is not listed in MANIFEST.json")
    for error in errors:
        print(f"verify_wasm: {error}", file=sys.stderr)
    if not errors:
        print(f"verify_wasm: {len(entries)} files match MANIFEST.json ({manifest['source']['tag']})")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
