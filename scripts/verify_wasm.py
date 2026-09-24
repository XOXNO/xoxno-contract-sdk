#!/usr/bin/env python3
"""Check the embedded WASM files against wasm/MANIFEST.json.

Fails when a file is missing, when its SHA-256 differs from the manifest, or
when a contract's file with docs and its deploy artifact do not have the code
hash recorded at sync time.
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
        elif "code_sha256" in entry and code_hash(data) != entry["code_sha256"]:
            errors.append(f"{name}: code hash differs from MANIFEST.json")
        if "deploy_file" in entry:
            deploy = WASM_DIR / entry["deploy_file"]
            if not deploy.is_file():
                errors.append(f"{name}: {entry['deploy_file']} is missing")
                continue
            data = deploy.read_bytes()
            if hashlib.sha256(data).hexdigest() != entry["deploy_sha256"]:
                errors.append(f"{name}: deploy artifact sha256 differs from MANIFEST.json")
            elif code_hash(data) != entry["code_sha256"]:
                errors.append(f"{name}: deploy artifact code differs from MANIFEST.json")
    listed = {entry["file"] for entry in entries.values()}
    listed |= {entry["deploy_file"] for entry in entries.values() if "deploy_file" in entry}
    for path in WASM_DIR.rglob("*.wasm"):
        if path.relative_to(WASM_DIR).as_posix() not in listed:
            errors.append(f"{path.relative_to(WASM_DIR)} is not listed in MANIFEST.json")
    for error in errors:
        print(f"verify_wasm: {error}", file=sys.stderr)
    if not errors:
        print(f"verify_wasm: {len(entries)} files match MANIFEST.json ({manifest['source']['tag']})")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
