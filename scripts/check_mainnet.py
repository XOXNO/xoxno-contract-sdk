#!/usr/bin/env python3
"""Check that every embedded contract is still the code deployed on mainnet.

Reads each contract instance's WASM hash from the mainnet RPC and compares it
with `deploy_sha256` in wasm/MANIFEST.json. A difference means the SDK snapshot differs
from mainnet and needs review before release. The price aggregator address is resolved
from the controller with a read-only simulation, so replacements are checked too.

Usage: check_mainnet.py [--rpc-url URL]
"""
import argparse
import json
import sys

from sync_wasm import WASM_DIR, live_price_aggregator, live_wasm_hash

PASSPHRASE = "Public Global Stellar Network ; September 2015"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rpc-url", default="https://stellar-gateway.xoxno.com")
    parser.add_argument("--allow-undeployed", action="store_true",
                        help="report hash drift without failing; RPC errors still fail")
    args = parser.parse_args()

    manifest = json.loads((WASM_DIR / "MANIFEST.json").read_text())
    cfg = {"rpc_url": args.rpc_url, "network_passphrase": PASSPHRASE}
    aggregator = live_price_aggregator(manifest["contracts"]["controller"]["mainnet_contract"], cfg)
    stale = []
    for name, entry in manifest["contracts"].items():
        address = aggregator if name == "price_aggregator" else entry["mainnet_contract"]
        live = live_wasm_hash(address, cfg)
        status = "ok" if live == entry["deploy_sha256"] else "DIFFERS"
        print(f"{name:17} {status:8} sdk {entry['deploy_sha256'][:12]} mainnet {live[:12]}")
        if address != entry["mainnet_contract"]:
            print(f"{'':17} active address {address} (replaced {entry['mainnet_contract']})")
        if live != entry["deploy_sha256"]:
            stale.append(name)
    if stale:
        print(f"check_mainnet: mainnet runs different code for {', '.join(stale)}; sync and release the SDK",
              file=sys.stderr)
    return 1 if stale and not args.allow_undeployed else 0


if __name__ == "__main__":
    sys.exit(main())
