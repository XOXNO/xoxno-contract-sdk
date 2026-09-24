# xoxno-contract-sdk

Rust SDK for Soroban contracts that integrate XOXNO Lending. The published
crate is [`crates/xoxno-contract-sdk`](crates/xoxno-contract-sdk); its
[README](crates/xoxno-contract-sdk/README.md) is the user guide.

| Path | Contents |
|---|---|
| `crates/xoxno-contract-sdk` | The published crate: clients, types, WASM, helpers and the `testutils` fixture |
| `crates/xoxno-contract-sdk/wasm` | Embedded contract WASM and `MANIFEST.json` |
| `examples/lending-vault` | Example integrator contract and its tests (not published) |
| `scripts` | WASM sync, manifest and mainnet checks |

## Development

```bash
cargo test --workspace --all-features
```

The other CI checks:

```bash
python3 scripts/verify_wasm.py
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build -p xoxno-contract-sdk --target wasm32v1-none
cargo build -p lending-vault --target wasm32v1-none --release
```

## Update the embedded WASM

The WASM comes from an attested
[rs-lending-xlm](https://github.com/XOXNO/rs-lending-xlm) release, and it must
be the code that runs on mainnet:

```bash
python3 scripts/sync_wasm.py --release <rs-lending-xlm tag>
```

The script:

- verifies each file's build attestation;
- checks that its code equals the stripped deploy artifact, and that the
  artifact hash equals the live mainnet contract hash;
- writes `wasm/MANIFEST.json` and `src/networks.rs`.

`--build-dir` takes a local rs-lending-xlm checkout after
`make build deploy-artifacts` instead. `--allow-undeployed` accepts code that
is not on mainnet yet, and is for pre-releases only.

The `Mainnet drift` workflow runs `scripts/check_mainnet.py` every day. It
fails when mainnet runs code that the SDK does not embed.

## Release

1. Sync the WASM if mainnet changed. Bump the version in
   `crates/xoxno-contract-sdk/Cargo.toml` and in the workspace `Cargo.toml`
   dependency entry. Add the version's section to `CHANGELOG.md`.
2. Merge to `main`.
3. Run the `Publish` workflow with the version. It checks the version, the
   changelog and the WASM, runs the tests, publishes to crates.io with the
   `CRATES_IO_TOKEN` secret, and tags and releases `vX.Y.Z`.

## License

MIT. See [LICENSE](LICENSE).
