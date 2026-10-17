# SDK integration coverage

The manual workflow requires three independent lanes. Each lane has a fixed set
of mandatory cases; any missing case, failed assertion, unknown transaction,
remaining account or token residue fails its report. This is flow coverage, not a
claim of 100% Rust line/branch coverage or a substitute for protocol auditing.

| Surface | Shared testnet | Isolated testnet | Deterministic protocol-WASM tests |
| --- | --- | --- | --- |
| Single deposit/supply/borrow/repay/withdraw | `run.py`: six baseline examples; exact debit/payout, refunds, NFT close/reopen | — | SDK `wrapper.rs`, example tests |
| `deposit_batch`, `supply_batch` | Multiple tokens, repeated market, same token in hubs 1 and 2; debit and principal | Fixture liquidity and victim setup | `advanced-account/tests/flows.rs`: strict owner auth, duplicate sums, atomic rollback |
| `borrow_batch`, `repay_batch` | Multiple tokens/hubs, duplicate repayment, partial/full, cap refunds | Migration/liquidation cleanup | SDK `wrapper.rs`, `accounting.rs`: invalid legs, ambiguous token/hub, receipt taxes |
| `withdraw_batch` | Partial amounts, unsafe full exit rejection, sentinel close, NFT burn, zero residue | Borrower then liquidity recovery | Duplicate market/sentinel merge and closure |
| `liquidate_batch` | Healthy account rejection with unchanged funds/principal/NFT | Unhealthy victim; two payment tokens, duplicate offer, measured spend and seized collateral | SDK `accounting_edges.rs`: partial/full/insolvent plans, unused legs, negative net spend |
| `multiply` | Multiply, Long, Short; initial collateral, real quote routes | — | Debt-token initial payment, third-token conversion, same token in different hubs |
| `swap_debt` | New debt routed into existing debt; debt decreases/increases in correct markets | — | Protocol-WASM router fixture |
| `swap_collateral` | Current collateral routed into new collateral; both legs verified | — | Protocol-WASM router fixture |
| `repay_debt_with_collateral` | Routed partial repayment; same-market empty-route partial and full close | — | Protocol-WASM partial and routed full-close/refund tests |
| `flash_position` | Separate SDK receiver, actual receipt, measured collateral, NFT/debt backing, cleanup | — | Forged direct callback rejection; collateral-minimum rollback; real controller callback authorization |
| `migrate_from_blend` | Shared governance untouched; its pool approval is not assumed | Actual Blend pool; collateral + uncollateralized supply + debt; source maps empty, destination principal, unused cap reduces hub debt | Funding transfer aggregation regression against faithful Blend mock |
| `flash_loan` | Real controller/pool callback, exact principal/fee recovery | — | SDK callback tests |
| Account and market views | Baseline and advanced outcomes cross-check direct controller/NFT/aggregator | Fixture linkage and code hashes | SDK `wrapper.rs`, `views.rs`: placement, rates, liquidity, TTL and prices |
| Rejections | Enforced attacker simulation, zero/negative/unlisted/unsafe batch, invalid route; full token/principal/NFT snapshots | Governance targets restricted to freshly deployed fixture | Receipt corruption, wrong network, missing cases, unknown outcomes, cleanup-key retention |

All facade writes are represented: `open_account`, `deposit`, `deposit_batch`,
`supply`, `supply_batch`, `borrow`, `borrow_batch`, `repay`, `repay_batch`,
`withdraw`, `withdraw_all`, `withdraw_batch`, `renew_account`, `flash_loan`,
`liquidate`, `liquidate_batch`. Strategy calls use the SDK's generated controller
client; no second handwritten controller ABI is introduced.

Fixture routes use a deterministic router to isolate financial/auth behavior.
Live routes come from the quote service. Both use actual embedded or deployed
protocol contracts. Fixture oracle controls operate only on the newly deployed
protocol. No live case is substituted with a fixture result.

Limits: mainnet execution, every generated administrative endpoint, arbitrary
oracle/provider combinations, and exhaustive callback/token adversaries are not
covered by these SDK lanes. Receipt-tax and oracle stale/future/deviation cases
are fixture tests. Keep protocol E2E and security tests alongside these checks.
