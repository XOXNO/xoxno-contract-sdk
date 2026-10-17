# SDK examples on lending testnet

This lane builds and deploys all six Rust SDK example contracts, then executes them
against the **existing** XOXNO Lending testnet. It checks the SDK's contract
integration, including contract authorization, actual token payments and account
lifecycle. The protocol repository's E2E lane deploys its own protocol; this lane
does not.

Run **Actions → SDK testnet E2E → Run workflow**. No secrets or pre-funded wallets
are needed. The workflow uses fresh Friendbot accounts and never changes shared
protocol governance, market parameters or oracle configuration. It runs manually
to avoid consuming shared testnet capacity on every PR.

Locally, with Rust 1.95, Python 3, curl, OpenSSL 3+ and Stellar CLI 28.0.0:

```sh
stellar contract build --locked --out-dir target/testnet-e2e-wasm
python3 -m unittest discover -s tests/e2e -p 'test_*.py'
python3 tests/e2e/run.py
```

The CLI installer is `.github/scripts/install-stellar-cli.sh`. Run from the SDK
repository. The runner prints the report path and exits nonzero on any missing
case, assertion failure, unresolved transaction or cleanup failure.

## What must pass

| Example | Live checks |
| --- | --- |
| `account-basics` | Deposit, same-account top-up, collateral amount, NFT owner, borrow payout and bounded debt, health/headroom, buffered repay/refund, no debt/residue, full withdrawal, account deletion/NFT burn, reopen with a new id |
| `simple-deposit` | Supply and owner recovery; principal, placement, NFT ownership and closure |
| `market-picker` | Live hub/spoke selection and market data, then the same supply/recovery checks |
| `lending-vault` | Supply/balance/recovery, unused flash-fee sweep, and a real controller → pool → SDK callback flash loan with principal plus fee repaid |
| `price-reader` | Strict/safe/diagnostic price agreement with the direct aggregator; one XLM has the correct USD value at 7 decimals |
| `liquidator` | Healthy account returns zero, owner sweep pays correctly, wrapper retains no funds |

Owner-only borrow, withdrawal and sweep calls also undergo **enforced RPC
simulation** from an attacker account, without owner signatures. These must fail
with an authorization error and leave balances unchanged. They are not committed
failed transactions. Owner/payer and attacker identities use separate private
CLI directories so automatic signing cannot import the owner into the attacker
checks.

Amounts use native testnet XLM, hub/spoke `1/1`; each initial supply is 100 XLM and
the borrow is 5 XLM. A distinct fee payer keeps fee/refund accounting out of owner
token assertions. Deposits/withdrawals allow two stroops of share-conversion
rounding. Debt may grow by at most 1% during the short borrow check; repayment
offers 1% above current debt to cover intervening interest and checks the refund.
Prices must agree during a stable quote window; only read comparisons may repeat.

## Evidence and failure behavior

`testnet.json` pins the target deployment, XLM and hub/spoke. Before mutations the
runner verifies the exact testnet passphrase through `getNetwork`. It reads the
controller's actual pool, NFT and aggregator links, code hashes and live market
flags. Governance changes, a reset deployment, unavailable Friendbot, stale prices,
full caps or missing liquidity fail the run; skipped flows never count as passing.

Every call is built once, simulated, authorized by the owner, simulated again in
enforced mode, and signed by the payer. Platform OpenSSL signs the Soroban auth;
private key material stays in the private directory. Before submission the runner
saves the exact signed envelope and hash, plus the parsed request and CLI output. The runner reconciles that hash with `getTransaction`; it never retries a
submitted mutation. It verifies the committed envelope hash, payer, arguments,
result and return/events success hash, then asserts balances and protocol state.
Each uploaded WASM is bound to its committed request and returned hash. Each new deployment's
live WASM hash must match the exact optimized build, with one explicit salt shared
by the build-only request and submission.

The artifact contains `report.json`, public receipts/views/logs and deployed WASM.
`report.json` records source/tool hashes, deployment addresses, cases, errors,
remaining accounts and unknown outcomes. Private CLI keys live outside the report
in temporary directories and are never uploaded. Successful cleanup deletes the keys.
A cleanup failure, unresolved outcome or remaining account keeps recovery keys locally; the report
names that private directory, which must never be included in an artifact. Hosted
runners discard local keys when the job ends.

On a known failure, the runner tries to recover unused fees and liquidator balances, then repay and close its own accounts. An unknown transaction outcome blocks further mutations, including
cleanup, and preserves the hash for manual inspection. Hard cancellation can also
leave positions behind; workflow concurrency does not cancel an active run. All
funds are disposable Friendbot XLM. The vault owner can sweep unused flash fees after a rejected loan; the runner
tests this recovery and uses it during cleanup.

This lane does not cover successful liquidation of an unhealthy account, swaps,
Blend migration, multiple-token batches, fee-on-transfer tokens, or mainnet. Those
need separate fixtures/markets; forcing shared oracle prices would make this lane
unsafe for other testnet users. Keep local fixture tests and protocol E2E alongside
this compatibility check.
