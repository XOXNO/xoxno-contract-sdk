#!/usr/bin/env python3
"""Check that every embedded contract is still the code deployed on mainnet.

Reads each contract instance's WASM hash from the mainnet RPC and compares it
with `deploy_sha256` in wasm/MANIFEST.json. A difference means mainnet was
upgraded and a new SDK release is due. It checks the contract addresses in
MANIFEST.json; a price aggregator that governance replaced shows up only after
the next sync.

Usage: check_mainnet.py [--rpc-url URL]
"""
import argparse
import json
import sys

from sync_wasm import WASM_DIR, live_wasm_hash

PASSPHRASE = "Public Global Stellar Network ; September 2015"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rpc-url", default="https://stellar-gateway.xoxno.com")
    args = parser.parse_args()

    manifest = json.loads((WASM_DIR / "MANIFEST.json").read_text())
    cfg = {"rpc_url": args.rpc_url, "network_passphrase": PASSPHRASE}
    stale = []
    for name, entry in manifest["contracts"].items():
        live = live_wasm_hash(entry["mainnet_contract"], cfg)
        status = "ok" if live == entry["deploy_sha256"] else "UPGRADED"
        print(f"{name:17} {status:8} sdk {entry['deploy_sha256'][:12]} mainnet {live[:12]}")
        if live != entry["deploy_sha256"]:
            stale.append(name)
    if stale:
        print(f"check_mainnet: mainnet runs new code for {', '.join(stale)}; sync and release the SDK",
              file=sys.stderr)
    return 1 if stale else 0


if __name__ == "__main__":
    sys.exit(main())
