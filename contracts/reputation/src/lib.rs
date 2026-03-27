#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, PartialEq)]
pub enum Role { Client, Freelancer }

#[contracttype]
#[derive(Clone)]
pub struct ReputationScore {
    pub address: Address,
    pub role: Role,
    /// Score in basis points (0–10000 = 0–100%)
    pub score: i32,
    pub total_jobs: u32,
}

#[contracttype]
pub enum DataKey { Score(Address, Role), Admin }

#[contract]
pub struct ReputationContract;

#[contractimpl]
impl ReputationContract {
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Update reputation after a completed job. `delta` in basis points.
    /// Score is clamped to [0, 10000].
    pub fn update_score(env: Env, address: Address, role: Role, delta: i32) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        let mut reputation = Self::get_score(env.clone(), address, role.clone());
        reputation.score = Self::clamp_score(reputation.score.saturating_add(delta));
        reputation.total_jobs = reputation.total_jobs.saturating_add(1);

        env.storage()
            .persistent()
            .set(&DataKey::Score(reputation.address.clone(), role), &reputation);
    }

    /// Slash address for fraud / abandonment — reduces score by 20%.
    pub fn slash(env: Env, address: Address, role: Role, reason: Symbol) {
        let _ = reason;
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        let mut reputation = Self::get_score(env.clone(), address, role.clone());
        reputation.score = Self::clamp_score(reputation.score.saturating_sub(2000));

        env.storage()
            .persistent()
            .set(&DataKey::Score(reputation.address.clone(), role), &reputation);
    }

    pub fn get_score(env: Env, address: Address, role: Role) -> ReputationScore {
        env.storage()
            .persistent()
            .get(&DataKey::Score(address.clone(), role.clone()))
            .unwrap_or(ReputationScore {
                address,
                role,
                score: 5000,
                total_jobs: 0,
            })
    }
}

impl ReputationContract {
    fn clamp_score(value: i32) -> i32 {
        value.clamp(0, 10_000)
    }
}
