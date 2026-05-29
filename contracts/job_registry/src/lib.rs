#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec, Bytes};

/* -----------------------------------------------------------------
   1. State Configurations & Schema Definitions
----------------------------------------------------------------- */

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStatus {
    AwaitingFunding,
    Assigned,
    Completed,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    ClientVerified(Address),
    JobConfig(u64),
    JobBids(u64),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobConfig {
    pub creator: Address,
    pub ipfs_cid: Bytes,
    pub budget: i128,
    pub status: JobStatus,
    pub freelancer: Option<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bid {
    pub bidder: Address,
    pub amount: i128,
    pub timestamp: u64,
}

/* -----------------------------------------------------------------
   2. Explicit Event Schemas for Indexer & Storage Sync
----------------------------------------------------------------- */

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobStorageReclaimedEvent {
    pub job_id: u64,
    pub reclaimer: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobCreatedIndexEvent {
    pub job_id: u64,
    pub creator: Address,
    pub ipfs_cid: Bytes,
    pub budget: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobAssignedIndexEvent {
    pub job_id: u64,
    pub freelancer: Address,
    pub final_amount: i128,
}

/* -----------------------------------------------------------------
   3. Smart Contract Implementation
----------------------------------------------------------------- */

#[contract]
pub struct LanceJobRegistryContract;

#[contractimpl]
impl LanceJobRegistryContract {

    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Registry already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    pub fn set_client_verification(env: Env, client: Address, status: bool) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).expect("Registry uninitialized");
        admin.require_auth();
        env.storage().persistent().set(&DataKey::ClientVerified(client), &status);
    }

    /// Post a new job posting entry using a compact IPFS CID.
    pub fn post_job(env: Env, job_id: u64, creator: Address, ipfs_cid: Bytes, budget: i128) {
        creator.require_auth();

        let verification_key = DataKey::ClientVerified(creator.clone());
        let is_verified = env.storage().persistent().get(&verification_key).unwrap_or(false);
        if !is_verified {
            panic!("Identity constraint violation: Client profile must be verified");
        }

        if budget <= 0 {
            panic!("Budget parameters must be positive value");
        }
        if ipfs_cid.len() < 32 {
            panic!("Invalid IPFS Content Identifier bounds");
        }

        let job_key = DataKey::JobConfig(job_id);
        if env.storage().persistent().has(&job_key) {
            panic!("Job ID identifier collision detected");
        }

        let config = JobConfig {
            creator: creator.clone(),
            ipfs_cid: ipfs_cid.clone(),
            budget,
            status: JobStatus::AwaitingFunding,
            freelancer: None,
        };

        env.storage().persistent().set(&job_key, &config);
        
        let bids_key = DataKey::JobBids(job_id);
        let empty_bids: Vec<Bid> = Vec::new(&env);
        env.storage().persistent().set(&bids_key, &empty_bids);

        env.events().publish(
            (Symbol::new(&env, "job_posted"), job_id),
            JobCreatedIndexEvent { job_id, creator, ipfs_cid, budget },
        );
    }

    /// Accepts a proposal. Automatically transitions state machine parameters to 'Assigned'.
    pub fn accept_bid(env: Env, job_id: u64, bid_index: u32) {
        let job_key = DataKey::JobConfig(job_id);
        let mut job: JobConfig = env.storage().persistent().get(&job_key).expect("Job context not found");

        job.creator.require_auth();

        if job.status != JobStatus::AwaitingFunding {
            panic!("Job state already locked or assigned");
        }

        let bids_key = DataKey::JobBids(job_id);
        let bids: Vec<Bid> = env.storage().persistent().get(&bids_key).expect("Bids store missing");

        if bid_index >= bids.len() {
            panic!("Out-of-bounds input error: Selected bid index does not exist");
        }

        let chosen_bid = bids.get(bid_index).unwrap();

        job.status = JobStatus::Assigned;
        job.freelancer = Some(chosen_bid.bidder.clone());

        env.storage().persistent().set(&job_key, &job);

        env.events().publish(
            (Symbol::new(&env, "job_assigned"), job_id),
            JobAssignedIndexEvent {
                job_id,
                freelancer: chosen_bid.bidder,
                final_amount: chosen_bid.amount,
            },
        );
    }

    /// Admin or Creator capability to mark a finalized job as completed.
    pub fn complete_job(env: Env, job_id: u64) {
        let job_key = DataKey::JobConfig(job_id);
        let mut job: JobConfig = env.storage().persistent().get(&job_key).expect("Job context not found");
        
        job.creator.require_auth();

        if job.status != JobStatus::Assigned {
            panic!("Only active assigned jobs can be closed or completed");
        }

        job.status = JobStatus::Completed;
        env.storage().persistent().set(&job_key, &job);
    }

    /// Explicit Storage Reclamation System.
    /// Permanently expunges closed/completed postings to free storage keys and reclaim rent allocations.
    pub fn reclaim_job_storage(env: Env, job_id: u64, reclaimer: Address) {
        reclaimer.require_auth();

        let job_key = DataKey::JobConfig(job_id);
        let job: JobConfig = env.storage().persistent().get(&job_key).expect("Job context not found");

        // Safety enforcement verification boundaries
        if job.status != JobStatus::Completed {
            panic!("Storage optimization block: Only completed jobs can have their footprints reclaimed");
        }
        if reclaimer != job.creator {
            panic!("Unauthorized: Only the initial job creator can invoke storage reclamation");
        }

        let bids_key = DataKey::JobBids(job_id);

        // Safely purge persistent keys completely from storage ledger allocation tables
        env.storage().persistent().remove(&job_key);
        env.storage().persistent().remove(&bids_key);

        // Emit indexer synchronization notification event
        env.events().publish(
            (Symbol::new(&env, "job_storage_reclaimed"), job_id),
            JobStorageReclaimedEvent { job_id, reclaimer },
        );
    }

    /* -----------------------------------------------------------------
       Public Getters
    ----------------------------------------------------------------- */

    pub fn get_job(env: Env, job_id: u64) -> Option<JobConfig> {
        env.storage().persistent().get(&DataKey::JobConfig(job_id))
    }
}
