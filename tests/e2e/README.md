# SDK examples on lending testnet

The workflow builds seven Rust SDK example contracts and runs three required lanes.
`run.py` executes the six baseline examples against the **existing** XOXNO Lending testnet.
`advanced.py` exercises batches and routed strategies there. `controlled.py` deploys
a separate protocol from the SDK-attested WASM for liquidation and real Blend migration. It checks the SDK's contract
integration, including contract authorization, actual token payments and account
lifecycle. The [coverage inventory](COVERAGE.md) distinguishes live proof from fixture tests.

Run **Actions → SDK testnet E2E → Run workflow**. No secrets or pre-funded wallets
are needed. One disposable Friendbot owner funds a separate payer and attacker
through verified classic account-creation receipts. If Friendbot times out, setup
checks the new owner on-chain before retrying. The workflow never changes shared
protocol governance, market parameters or oracle configuration. It runs manually
to avoid consuming shared testnet capacity on every PR.

Locally, with Rust 1.95, Python 3, curl, OpenSSL 3+ and Stellar CLI 28.0.0:

```sh
stellar contract build --locked --out-dir target/testnet-e2e-wasm
python3 -m unittest discover -s tests/e2e -p 'test_*.py'
python3 tests/e2e/run.py
python3 tests/e2e/advanced.py
python3 tests/e2e/controlled.py
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
Read/build/simulation requests may retry transient transport errors; submitted
transactions never retry.
Before payer signing, the harness adds resource-fee headroom of 0.01 XLM or
50% of the estimate, whichever is larger. Unused refundable fees return to the payer.
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

The advanced lane uses native XLM and USDC across hubs 1/2 in spoke 3. It funds
USDC with a real router quote, creates an owner trustline, and recovers every
account and touched token. Testnet AMM prices can differ greatly from oracle
prices, so strategy draws are small relative to initial equity.

The controlled lane owns a fresh governance/controller/pool/NFT/aggregator and
mock oracles, using the SDK's embedded deployment WASM. It approves the real
Blend pool only in that deployment. Blend's test token addresses differ from
this protocol's USDC; migration therefore uses the common native XLM reserve.
Unused migration caps repay the new hub debt; they are not promised as owner
cash refunds. Shared testnet governance and oracle prices are never changed.

These scripts are examples for exact-transfer tokens, owned entirely by one
owner, with no depositor shares. The flash-position receiver deliberately uses
its prefunded collateral as equity and authenticates both controller and initiator.
The examples are not published inside the SDK crate.
