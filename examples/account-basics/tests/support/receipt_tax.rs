use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

// Test-only token: burns 10% of transfers to the chosen recipient.
#[contracttype]
enum Key {
    Balance(Address),
    TaxTo,
}

#[contract]
pub struct ReceiptTaxToken;

#[contractimpl]
impl ReceiptTaxToken {
    pub fn __constructor(env: Env, tax_to: Address) {
        env.storage().instance().set(&Key::TaxTo, &tax_to);
    }

    pub fn decimals() -> u32 {
        7
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let balance = Self::balance(env.clone(), to.clone());
        env.storage()
            .instance()
            .set(&Key::Balance(to), &(balance + amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage().instance().get(&Key::Balance(id)).unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let balance = Self::balance(env.clone(), from.clone());
        assert!(amount >= 0 && balance >= amount);
        env.storage()
            .instance()
            .set(&Key::Balance(from), &(balance - amount));
        let tax_to: Address = env.storage().instance().get(&Key::TaxTo).unwrap();
        let received = if to == tax_to {
            amount - amount / 10
        } else {
            amount
        };
        let balance = Self::balance(env.clone(), to.clone());
        env.storage()
            .instance()
            .set(&Key::Balance(to), &(balance + received));
    }
}
