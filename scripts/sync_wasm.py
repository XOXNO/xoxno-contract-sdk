#!/usr/bin/env python3
"""Copy a verified XOXNO Lending build into the crate.

Writes crates/xoxno-contract-sdk/wasm/*.wasm, wasm/MANIFEST.json and
src/networks.rs.

Two sources:
  --release TAG      download the attested SDK bundle of an rs-lending-xlm
                     GitHub release (sdk-*.wasm, sdk-manifest.json)
  --build-dir PATH   use an rs-lending-xlm checkout on which
                     `make build deploy-artifacts` has run

Every contract must have the same code as the stripped deploy artifact, and
that artifact's hash must equal the WASM hash of the live mainnet contract
instance, read through the mainnet RPC in configs/networks.json. Pass
--allow-undeployed only for a pre-release.
"""
import argparse
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

from wasm_code_hash import code_hash

REPO = "XOXNO/rs-lending-xlm"
ROOT = pathlib.Path(__file__).resolve().parent.parent
CRATE = ROOT / "crates" / "xoxno-contract-sdk"
WASM_DIR = CRATE / "wasm"
NETWORKS_RS = CRATE / "src" / "networks.rs"

CONTRACTS = ["controller", "pool", "position_nft", "price_aggregator", "governance"]
MOCKS = {"mock_reflector": "mock_oracle", "mock_redstone": "mock_redstone"}
NETWORKS = ["mainnet", "testnet"]


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run(*args: str) -> str:
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def fail(message: str) -> None:
    sys.exit(f"sync_wasm: {message}")


def from_build_dir(build: pathlib.Path) -> tuple[dict, dict, dict]:
    """Returns (files, deploy_hashes, source) from a local checkout."""
    files, deploy = {}, {}
    for name in CONTRACTS:
        files[name] = (build / "target" / "optimized" / f"{name}.wasm").read_bytes()
        deploy[name] = sha256((build / "artifacts" / "wasm" / "deploy" / f"{name}.wasm").read_bytes())
        if code_hash(files[name]) != code_hash((build / "artifacts" / "wasm" / "deploy" / f"{name}.wasm").read_bytes()):
            fail(f"{name}: optimized build and deploy artifact have different code")
    release = build / "target" / "wasm32v1-none" / "release"
    for sdk_name, crate_name in MOCKS.items():
        files[sdk_name] = (release / f"{crate_name}.wasm").read_bytes()
    commit = run("git", "-C", str(build), "rev-parse", "HEAD").strip()
    tag = run("git", "-C", str(build), "describe", "--tags", "--always").strip()
    return files, deploy, {"tag": tag, "commit": commit}


def from_release(tag: str) -> tuple[dict, dict, dict]:
    """Returns (files, deploy_hashes, source) from an attested release."""
    with tempfile.TemporaryDirectory() as tmp:
        run("gh", "release", "download", tag, "-R", REPO, "-D", tmp, "-p", "sdk-*")
        out = pathlib.Path(tmp)
        run("gh", "attestation", "verify", str(out / "sdk-manifest.json"), "--repo", REPO)
        manifest = json.loads((out / "sdk-manifest.json").read_text())
        files, deploy = {}, {}
        for name in CONTRACTS + list(MOCKS):
            path = out / f"sdk-{name}.wasm"
            run("gh", "attestation", "verify", str(path), "--repo", REPO)
            files[name] = path.read_bytes()
            entry = manifest[name]
            if sha256(files[name]) != entry["sha256"]:
                fail(f"{name}: file hash differs from sdk-manifest.json")
            if name in CONTRACTS:
                if code_hash(files[name]) != entry["code_sha256"]:
                    fail(f"{name}: code hash differs from sdk-manifest.json")
                deploy[name] = entry["deploy_sha256"]
    commit = run("gh", "api", f"repos/{REPO}/commits/{tag}", "--jq", ".sha").strip()
    return files, deploy, {"tag": tag, "commit": commit}


def load_config(config_dir: "pathlib.Path | None", ref: str) -> dict:
    """Returns the deployment configuration: networks.json and each network's spokes.json."""
    if config_dir is not None:
        def read(path: str) -> dict:
            return json.loads((config_dir / path).read_text())
    else:
        def read(path: str) -> dict:
            return json.loads(run("gh", "api", f"repos/{REPO}/contents/{path}?ref={ref}",
                                  "-H", "Accept: application/vnd.github.raw"))
    return {
        "networks": read("configs/networks.json"),
        "spokes": {net: read(f"configs/{net}/spokes.json") for net in NETWORKS},
    }


def live_wasm_hash(contract: str, cfg: dict) -> str:
    out = run("stellar", "ledger", "entry", "fetch", "contract-data", "--contract", contract,
              "--instance", "--rpc-url", cfg["rpc_url"], "--network-passphrase",
              cfg["network_passphrase"], "--output", "json")
    found = re.findall(r'"wasm"\s*:\s*"([0-9a-f]{64})"', out)
    if len(found) != 1:
        fail(f"{contract}: cannot read the instance WASM hash from RPC")
    return found[0]


def check_deployed(deploy: dict, networks: dict, allow_undeployed: bool) -> None:
    mainnet = networks["mainnet"]
    for name, digest in deploy.items():
        live = live_wasm_hash(mainnet[name], mainnet)
        if live != digest:
            message = f"{name}: deploy hash {digest[:12]} is not the live mainnet hash {live[:12]}"
            if not allow_undeployed:
                fail(message + " (pass --allow-undeployed for a pre-release)")
            print(f"warning: {message}", file=sys.stderr)


def write_manifest(files: dict, deploy: dict, source: dict) -> None:
    WASM_DIR.mkdir(parents=True, exist_ok=True)
    manifest = {
        "source": {"repository": REPO, "tag": source["tag"], "commit": source["commit"]},
        "contracts": {},
        "mocks": {},
    }
    mainnet = source["networks"]["mainnet"]
    for name, data in files.items():
        (WASM_DIR / f"{name}.wasm").write_bytes(data)
        entry = {"file": f"{name}.wasm", "sha256": sha256(data)}
        if name in CONTRACTS:
            entry["code_sha256"] = code_hash(data)
            entry["deploy_sha256"] = deploy[name]
            entry["mainnet_contract"] = mainnet[name]
            manifest["contracts"][name] = entry
        else:
            manifest["mocks"][name] = entry
    (WASM_DIR / "MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n")


def write_networks(source: dict) -> None:
    lines = [
        "//! Stable contract addresses and on-chain ids of the XOXNO Lending deployments.",
        "//!",
        f"//! Generated by `scripts/sync_wasm.py` from the rs-lending-xlm deployment configuration at `{source['config_ref']}`.",
        "//! Do not edit by hand. Governance can add spokes and hubs later: read",
        "//! `get_spoke` on the controller for the live configuration.",
        "",
    ]
    for net in NETWORKS:
        cfg = source["networks"][net]
        names = source["spokes"][net]
        lines += [f"/// XOXNO Lending on Stellar {net}.", f"pub mod {net} {{"]
        for key, label in [
            ("governance", "Governance and timelock contract."),
            ("controller", "Controller: the entry point for every user operation."),
            ("pool", "Liquidity pool that holds every market's cash."),
            ("position_nft", "Position NFT: the token id is the account id; its owner owns the account."),
        ]:
            lines += [f"    /// {label}", f'    pub const {key.upper()}: &str = "{cfg[key]}";']
        hubs = sorted(set(cfg["hub_ids"].values()))
        lines += ["    /// On-chain hub ids.", f"    pub const HUB_IDS: &[u32] = &{hubs};"]
        lines += ["    /// On-chain spoke ids with the spoke names from the deployment configuration.",
                  "    pub const SPOKES: &[(u32, &str)] = &["]
        for key, onchain in sorted(cfg["spoke_ids"].items(), key=lambda kv: kv[1]):
            name = names.get(key, {}).get("name", "")
            lines.append(f'        ({onchain}, "{name}"),')
        lines += ["    ];", "}", ""]
    NETWORKS_RS.write_text("\n".join(lines).rstrip() + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--release", metavar="TAG")
    group.add_argument("--build-dir", type=pathlib.Path)
    parser.add_argument("--config-dir", type=pathlib.Path,
                        help="rs-lending-xlm checkout to read configs/ from (default: GitHub main)")
    parser.add_argument("--allow-undeployed", action="store_true")
    args = parser.parse_args()

    files, deploy, source = from_release(args.release) if args.release else from_build_dir(args.build_dir)
    source.update(load_config(args.config_dir, "main"))
    source["config_ref"] = (run("git", "-C", str(args.config_dir), "rev-parse", "--short", "HEAD").strip()
                            if args.config_dir else "main")
    check_deployed(deploy, source["networks"], args.allow_undeployed)
    if WASM_DIR.exists():
        shutil.rmtree(WASM_DIR)
    write_manifest(files, deploy, source)
    write_networks(source)
    print(f"synced {len(files)} files from {source['tag']} ({source['commit'][:9]})")


if __name__ == "__main__":
    main()
