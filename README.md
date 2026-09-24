# xoxno-contract-sdk

Rust SDK for Soroban contracts that integrate XOXNO Lending. The published
crate is [`crates/xoxno-contract-sdk`](crates/xoxno-contract-sdk); its
[README](crates/xoxno-contract-sdk/README.md) is the user guide.

| Path | Contents |
|---|---|
| `crates/xoxno-contract-sdk` | The published crate: the `XoxnoLending` wrapper, generated clients, types, WASM, helpers and the `testutils` fixture |
| `crates/xoxno-contract-sdk/wasm` | Embedded contract WASM and `MANIFEST.json` |
| `examples/*` | Example integrator contracts and their tests (not published): `account-basics`, `market-picker`, `price-reader`, `liquidator`, `lending-vault` |
| `scripts` | WASM sync, manifest and mainnet checks |

## Development

```bash
cargo test --workspace --all-features
```

The other CI checks:

```bash
python3 scripts/verify_wasm.py
python3 scripts/gen_clients.py --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
stellar contract build
```

`soroban-sdk` 28 builds contracts only through `stellar contract build`
(stellar-cli 25.2 or newer).

## Update the embedded WASM

The WASM comes from an attested
[rs-lending-xlm](https://github.com/XOXNO/rs-lending-xlm) release, and it must
be the code that runs on mainnet:

```bash
python3 scripts/sync_wasm.py --release <rs-lending-xlm tag>
```

The script:

- verifies each file's build attestation, signed by the rs-lending-xlm
  `release.yml` workflow;
- checks that its code equals the stripped deploy artifact, and that the
  artifact hash equals the live mainnet contract hash;
- writes `wasm/MANIFEST.json` and `src/networks.rs`.

`--build-dir` takes a clean local rs-lending-xlm checkout of a tag after
`make build deploy-artifacts` instead; 0.1.0 was synced this way from
`v1.0.0`, whose release predates the SDK bundle. `--allow-undeployed` accepts
code that is not on mainnet yet, and is for pre-releases only.

The `Mainnet drift` workflow runs `scripts/check_mainnet.py` every day. It
fails when mainnet runs code that the SDK does not embed.

## Release

1. Sync the WASM if mainnet changed. Bump the version in
   `crates/xoxno-contract-sdk/Cargo.toml` and in the workspace `Cargo.toml`
   dependency entry. Add the version's dated section to `CHANGELOG.md`
   (`## [X.Y.Z] - YYYY-MM-DD`).
2. Merge to `main`.
3. Run the `Publish` workflow on `main` with the version. It checks the
   version, the changelog, the WASM and the mainnet hashes, runs the tests,
   publishes to crates.io with the `CRATES_IO_TOKEN` secret, and creates the
   `vX.Y.Z` tag and GitHub release.

## License

MIT. See [LICENSE](LICENSE).
