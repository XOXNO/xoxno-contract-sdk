//! Authorization helpers for contracts that call XOXNO Lending.

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{symbol_short, token, vec, Address, Env, IntoVal, Vec};

/// Approves `pool` to pull `amount_plus_fee` of `token` from the current
/// contract, as a flash-loan receiver must do before
/// [`FlashLoanReceiver::execute_flash_loan`](crate::lending::FlashLoanReceiver::execute_flash_loan)
/// returns.
///
/// The approval expires at the next ledger.
pub fn approve_flash_repayment(env: &Env, token: &Address, pool: &Address, amount_plus_fee: i128) {
    let expiration_ledger = env.ledger().sequence().saturating_add(1);
    token::Client::new(env, token).approve(
        &env.current_contract_address(),
        pool,
        &amount_plus_fee,
        &expiration_ledger,
    );
}

/// Authorizes one `transfer(from, to, amount)` on `token` inside the next call
/// the current contract makes.
///
/// Use it before `supply` or `repay` when the current contract is the payer:
/// the controller moves the tokens with `transfer`, which is a sub-invocation
/// of the controller call and therefore needs this entry.
pub fn authorize_transfer_as_current(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) {
    env.authorize_as_current_contract(vec![env, transfer_entry(env, token, from, to, amount)]);
}

/// Authorizes one `transfer(from, to, amount)` per `(token, amount)` of
/// `transfers` inside the next call the current contract makes.
///
/// Use it before a controller call that pulls several tokens, such as a
/// `supply` or `repay` with several markets. The controller makes one
/// transfer per distinct market, in request order; each entry matches one
/// transfer, so two equal transfers need two entries.
pub fn authorize_transfers_as_current(
    env: &Env,
    from: &Address,
    to: &Address,
    transfers: &Vec<(Address, i128)>,
) {
    let mut entries = Vec::new(env);
    for (token, amount) in transfers.iter() {
        entries.push_back(transfer_entry(env, &token, from, to, amount));
    }
    env.authorize_as_current_contract(entries);
}

fn transfer_entry(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) -> InvokerContractAuthEntry {
    InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: token.clone(),
            fn_name: symbol_short!("transfer"),
            args: (from.clone(), to.clone(), amount).into_val(env),
        },
        sub_invocations: Vec::new(env),
    })
}
