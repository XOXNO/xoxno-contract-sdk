use soroban_sdk::{contractclient, Address, Bytes, Env};

/// Callback a contract must implement to receive a flash loan.
///
/// The pool calls it by name after it sends `amount` of `asset` to the
/// receiver. Implement it with `#[contractimpl] impl FlashLoanReceiver for
/// MyContract`.
///
/// Rules:
/// - Anyone can call this function directly with any arguments, and anyone can
///   start a flash loan to any receiver. Call `require_auth()` on the pool
///   address your contract stored at setup: it passes only when the pool is the
///   direct caller. Then check `initiator`, which the controller authenticated.
/// - Repay `amount + fee` by approving the pool as spender before the
///   callback returns; the pool then pulls the tokens with `transfer_from`.
///   Use [`approve_flash_repayment`](crate::lending::helpers::approve_flash_repayment).
///   Sending tokens to the pool directly makes the loan revert.
/// - The receiver must be a WASM contract; an account or a Stellar Asset
///   Contract is rejected.
/// - The receiver cannot be the contract that calls `flash_loan`, and the
///   callback cannot call the controller or the pool, not even their views:
///   Soroban rejects a call into a contract that is already on the call stack.
#[contractclient(name = "FlashLoanReceiverClient")]
pub trait FlashLoanReceiver {
    /// Receives the loan. `initiator` is the caller of `flash_loan` on the
    /// controller, `fee` is the fee due on top of `amount`, and `data` is the
    /// payload the initiator passed.
    fn execute_flash_loan(
        env: Env,
        initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        data: Bytes,
    );
}

/// Callback a contract must implement to receive a flash position.
///
/// The controller calls it by name after it opens the debt of
/// `flash_position`. Implement it with `#[contractimpl] impl
/// FlashPositionReceiver for MyContract`.
///
/// Rules:
/// - Anyone can call this function directly, and anyone can start a flash
///   position with any receiver: the collateral the receiver sends goes into
///   the account of the caller of `flash_position`. Call `require_auth()` on
///   the controller address your contract stored at setup, then check
///   `initiator`, which the controller authenticated.
/// - Send the collateral to the controller with a plain token `transfer`. The
///   controller measures its balance change for each declared collateral and
///   deposits it into the account. It does not pull tokens or accept an
///   approval. Declared refund assets go back to the caller.
/// - Tokens that were not declared as collateral or refund assets stay on the
///   controller and are lost to the caller.
/// - The receiver must be a WASM contract, and not the controller or the pool.
/// - The receiver cannot be the contract that calls `flash_position`, and the
///   callback cannot call the controller again.
#[contractclient(name = "FlashPositionReceiverClient")]
pub trait FlashPositionReceiver {
    /// Receives the borrowed `amount` of `asset` for `account_id`.
    /// `amount_received` is what reached the receiver, `fee` is always 0, and
    /// `data` is the payload the initiator passed.
    #[allow(clippy::too_many_arguments)]
    fn execute_flash_position(
        env: Env,
        initiator: Address,
        account_id: u64,
        asset: Address,
        amount: i128,
        fee: i128,
        amount_received: i128,
        controller: Address,
        data: Bytes,
    );
}
