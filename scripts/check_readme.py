#!/usr/bin/env python3
"""Compile every `rust` block of the crate README and run its tests.

Each block becomes one module of a throwaway no_std crate in
target/readme-check that depends on this crate with `testutils`. A block must
therefore be complete: its own imports, items only, no undefined names.

Usage: check_readme.py
"""
import pathlib
import re
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
README = ROOT / "crates" / "xoxno-contract-sdk" / "README.md"
CRATE = ROOT / "target" / "readme-check"

MANIFEST = f"""[package]
name = "readme-check"
version = "0.0.0"
edition = "2021"
publish = false

[workspace]

[dependencies]
soroban-sdk = "28.0.0"
xoxno-contract-sdk = {{ path = "{ROOT / 'crates' / 'xoxno-contract-sdk'}", features = ["testutils"] }}

[dev-dependencies]
soroban-sdk = {{ version = "28.0.0", features = ["testutils"] }}
"""


def main() -> int:
    blocks = re.findall(r"^```rust\n(.*?)^```$", README.read_text(), flags=re.M | re.S)
    if not blocks:
        sys.exit("check_readme: no rust blocks in README.md")
    (CRATE / "src").mkdir(parents=True, exist_ok=True)
    (CRATE / "Cargo.toml").write_text(MANIFEST)
    shutil.copyfile(ROOT / "Cargo.lock", CRATE / "Cargo.lock")
    modules = [f"pub mod block_{i} {{\n{block}}}\n" for i, block in enumerate(blocks, 1)]
    (CRATE / "src" / "lib.rs").write_text("#![no_std]\n#![deny(warnings)]\n\n" + "\n".join(modules))
    result = subprocess.run(["cargo", "test", "--quiet", "--manifest-path", str(CRATE / "Cargo.toml")])
    if result.returncode == 0:
        print(f"check_readme: {len(blocks)} blocks compile and their tests pass")
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
