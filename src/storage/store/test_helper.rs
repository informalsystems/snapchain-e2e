use crate::core::types::{Address, Vote};
use crate::mempool::mempool::MempoolMessagesRequest;
use crate::storage::db::{self, RocksDB};
use crate::storage::store::engine::ShardEngine;
use crate::storage::store::stores::StoreLimits;
use crate::storage::trie::merkle_trie;
use crate::utils::statsd_wrapper::StatsdClientWrapper;
use ed25519_dalek::{SecretKey, SigningKey};
use informalsystems_malachitebft_core_types::{NilOrVal, Round};
use libp2p::identity::ed25519::Keypair;
use prost::Message;
use rand::RngCore;
use std::sync::Arc;
use tempfile;
use tokio::sync::mpsc;

use crate::core::error::HubError;
use crate::core::util::FarcasterTime;
#[allow(unused_imports)] // Used by cfg(test)
use crate::proto::{self, FnameTransfer};
use crate::proto::{
    CommitSignature, Commits, Height, ShardChunk, ShardHash, ShardHeader, Transaction,
};
use crate::proto::{MessagesResponse, OnChainEvent};
use crate::storage::store::account::MessagesPage;
use crate::storage::store::engine::{MempoolMessage, ShardStateChange};
#[allow(unused_imports)] // Used by cfg(test)
use crate::storage::trie::merkle_trie::TrieKey;
use crate::storage::util::bytes_compare;
#[allow(unused_imports)]
use crate::utils::factory::{events_factory, username_factory};
use hex::FromHex;
use tonic::{Response, Status};
use tracing_subscriber::EnvFilter;

pub const FID_FOR_TEST: u64 = 1234;

#[cfg(test)]
pub const FID2_FOR_TEST: u64 = 1235;

#[cfg(test)]
pub const FID3_FOR_TEST: u64 = 2;

pub const SHARD1_FID: u64 = 121;
pub const SHARD2_FID: u64 = 122;

pub mod limits {
    use crate::storage::store::stores::Limits;

    pub fn zero() -> Limits {
        Limits {
            casts: 0,
            links: 0,
            reactions: 0,
            user_data: 0,
            user_name_proofs: 0,
            verifications: 0,
        }
    }

    pub fn one() -> Limits {
        Limits {
            casts: 1,
            links: 1,
            reactions: 1,
            user_data: 1,
            user_name_proofs: 1,
            verifications: 1,
        }
    }

    pub fn test() -> Limits {
        Limits {
            casts: 4,
            links: 4,
            reactions: 3,
            user_data: 4,
            user_name_proofs: 2,
            verifications: 2,
        }
    }

    // Slightly different, but still low limits for legacy units
    #[cfg(test)]
    pub fn legacy() -> Limits {
        Limits {
            casts: 10,
            links: 10,
            reactions: 5,
            user_data: 5,
            user_name_proofs: 5,
            verifications: 5,
        }
    }

    pub fn unlimited() -> Limits {
        Limits {
            casts: u32::MAX,
            links: u32::MAX,
            reactions: u32::MAX,
            user_data: u32::MAX,
            user_name_proofs: u32::MAX,
            verifications: u32::MAX,
        }
    }

    #[cfg(test)]
    pub fn test_store_limits() -> crate::storage::store::stores::StoreLimits {
        crate::storage::store::stores::StoreLimits {
            limits: test(),
            legacy_limits: legacy(),
        }
    }
}

pub struct EngineOptions {
    pub limits: Option<StoreLimits>,
    pub db: Option<Arc<RocksDB>>,
    pub messages_request_tx: Option<mpsc::Sender<MempoolMessagesRequest>>,
    pub network: Option<proto::FarcasterNetwork>,
    pub fname_signer_address: Option<alloy_primitives::Address>,
    pub shard_id: u32,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            limits: None,
            db: None,
            messages_request_tx: None,
            network: None,
            fname_signer_address: None,
            shard_id: 1,
        }
    }
}

pub fn statsd_client() -> StatsdClientWrapper {
    StatsdClientWrapper::new(
        cadence::StatsdClient::builder("", cadence::NopMetricSink {}).build(),
        true,
    )
}

pub fn new_engine_with_options(options: EngineOptions) -> (ShardEngine, tempfile::TempDir) {
    let statsd_client = statsd_client();
    let dir = tempfile::TempDir::new().unwrap();

    let db = match options.db {
        None => {
            let db_path = dir.path().join("test.db");

            let db = db::RocksDB::new(db_path.to_str().unwrap());
            db.open().unwrap();
            Arc::new(db)
        }
        Some(db) => db,
    };

    let test_limits = options.limits.unwrap_or(StoreLimits {
        limits: limits::test(),
        legacy_limits: limits::zero(),
    });

    (
        ShardEngine::new(
            db,
            options.network.unwrap_or(proto::FarcasterNetwork::Devnet), // So all protocol features are enabled by default
            merkle_trie::MerkleTrie::new(16).unwrap(),
            options.shard_id,
            test_limits,
            statsd_client,
            256,
            options.messages_request_tx,
            options.fname_signer_address,
        ),
        dir,
    )
}

#[cfg(test)]
pub fn new_engine() -> (ShardEngine, tempfile::TempDir) {
    new_engine_with_options(EngineOptions::default())
}

pub async fn commit_event(engine: &mut ShardEngine, event: &OnChainEvent) -> ShardChunk {
    let state_change = engine.propose_state_change(
        1,
        vec![MempoolMessage::ValidatorMessage(proto::ValidatorMessage {
            on_chain_event: Some(event.clone()),
            fname_transfer: None,
        })],
        None,
    );

    validate_and_commit_state_change(engine, &state_change)
}

pub async fn commit_event_at(
    engine: &mut ShardEngine,
    event: &OnChainEvent,
    timestamp: &FarcasterTime,
) -> ShardChunk {
    let state_change = engine.propose_state_change(
        1,
        vec![MempoolMessage::ValidatorMessage(proto::ValidatorMessage {
            on_chain_event: Some(event.clone()),
            fname_transfer: None,
        })],
        Some(timestamp.clone()),
    );
    validate_and_commit_state_change(engine, &state_change)
}

pub async fn sign_chunk(keypair: &Keypair, mut shard_chunk: ShardChunk) -> ShardChunk {
    let header = shard_chunk.header.as_ref().unwrap();
    let height = header.height.unwrap();
    let hash = ShardHash {
        hash: shard_chunk.hash.clone(),
        shard_index: height.shard_index,
    };
    let round = Round::from(0u32);

    let signer = keypair.public().to_bytes().to_vec();
    let address = Address::from_vec(signer.clone());

    let vote = Vote::new_precommit(height, round, NilOrVal::Val(hash.clone()), address);

    let signature = keypair.sign(&vote.to_sign_bytes());

    shard_chunk.commits = Some(Commits {
        height: Some(height),
        round: round.as_i64(),
        value: Some(hash),
        signatures: vec![CommitSignature { signature, signer }],
    });

    shard_chunk
}

#[cfg(test)]
pub async fn commit_message(engine: &mut ShardEngine, msg: &proto::Message) -> ShardChunk {
    let state_change =
        engine.propose_state_change(1, vec![MempoolMessage::UserMessage(msg.clone())], None);

    if state_change.transactions.is_empty() {
        panic!("Failed to propose message");
    }

    let chunk = validate_and_commit_state_change(engine, &state_change);
    assert_eq!(
        state_change.new_state_root,
        chunk.header.as_ref().unwrap().shard_root
    );
    assert!(engine.trie_key_exists(trie_ctx(), &TrieKey::for_message(msg)));
    chunk
}

// Note, this function does not check that the commit was successful, unlike `commit_message`.
pub async fn commit_message_at(
    engine: &mut ShardEngine,
    msg: &proto::Message,
    timestamp: &FarcasterTime,
) -> ShardChunk {
    let state_change = engine.propose_state_change(
        1,
        vec![MempoolMessage::UserMessage(msg.clone())],
        Some(timestamp.clone()),
    );

    if state_change.transactions.is_empty() {
        panic!("Failed to propose message");
    }

    let chunk = validate_and_commit_state_change(engine, &state_change);
    assert_eq!(
        state_change.new_state_root,
        chunk.header.as_ref().unwrap().shard_root
    );
    chunk
}

#[cfg(test)]
pub async fn commit_messages(engine: &mut ShardEngine, msgs: Vec<proto::Message>) -> ShardChunk {
    use itertools::Itertools;

    let state_change = engine.propose_state_change(
        1,
        msgs.iter()
            .map(|msg| MempoolMessage::UserMessage(msg.clone()))
            .collect_vec(),
        None,
    );

    if state_change.transactions.is_empty() {
        panic!("Failed to propose message");
    }

    let chunk = validate_and_commit_state_change(engine, &state_change);
    assert_eq!(
        state_change.new_state_root,
        chunk.header.as_ref().unwrap().shard_root
    );
    for msg in msgs {
        assert!(engine.trie_key_exists(trie_ctx(), &TrieKey::for_message(&msg)));
    }
    chunk
}

#[cfg(test)]
pub fn trie_ctx() -> &'static mut merkle_trie::Context<'static> {
    Box::leak(Box::new(merkle_trie::Context::new()))
}

#[cfg(test)]
pub fn message_exists_in_trie(engine: &mut ShardEngine, msg: &proto::Message) -> bool {
    engine.trie_key_exists(trie_ctx(), &TrieKey::for_message(msg))
}

#[cfg(test)]
pub fn key_exists_in_trie(engine: &mut ShardEngine, key: &Vec<u8>) -> bool {
    engine.trie_key_exists(trie_ctx(), key)
}

pub fn default_shard_chunk() -> ShardChunk {
    ShardChunk {
        header: Some(ShardHeader {
            height: Some(Height {
                shard_index: 1,
                block_number: 1,
            }),
            timestamp: 0,
            shard_root: vec![],
            parent_hash: vec![],
        }),
        // TODO: eventually we won't hardcode one transaction here
        transactions: vec![Transaction {
            user_messages: vec![],
            system_messages: vec![],
            fid: FID_FOR_TEST as u64,
            account_root: vec![5, 5, 6, 6], //TODO,
        }],
        hash: vec![],
        commits: None,
    }
}

pub fn state_change_to_shard_chunk(
    shard_index: u32,
    block_number: u64,
    change: &ShardStateChange,
) -> ShardChunk {
    let mut chunk = default_shard_chunk();

    chunk.header.as_mut().unwrap().shard_root = change.new_state_root.clone();
    chunk.header.as_mut().unwrap().height = Some(Height {
        shard_index,
        block_number,
    });
    chunk.header.as_mut().unwrap().timestamp = change.timestamp.clone().into();
    chunk.transactions = change.transactions.clone();
    chunk
}

pub fn validate_and_commit_state_change(
    engine: &mut ShardEngine,
    state_change: &ShardStateChange,
) -> ShardChunk {
    let height = engine.get_confirmed_height();
    engine.start_round(height.increment(), Round::Nil); // So event id is reset

    let valid = engine.validate_state_change(state_change);
    assert!(valid);

    let chunk = state_change_to_shard_chunk(1, height.block_number + 1, state_change);
    engine.commit_shard_chunk(&chunk);
    assert_eq!(state_change.new_state_root, engine.trie_root_hash());
    chunk
}

pub fn default_storage_event(fid: u64) -> OnChainEvent {
    events_factory::create_rent_event(fid, None, Some(1), false)
}

pub async fn register_user(
    fid: u64,
    signer: SigningKey,
    custody_address: Vec<u8>,
    engine: &mut ShardEngine,
) {
    commit_event(engine, &default_storage_event(fid)).await;
    let id_register_event = events_factory::create_id_register_event(
        fid,
        proto::IdRegisterEventType::Register,
        custody_address,
        None,
    );
    commit_event(engine, &id_register_event).await;
    let signer_event =
        events_factory::create_signer_event(fid, signer, proto::SignerEventType::Add, None, None);
    commit_event(engine, &signer_event).await;
}

#[cfg(test)]
pub async fn commit_fname_transfer(engine: &mut ShardEngine, transfer: &FnameTransfer) {
    let state_change = engine.propose_state_change(
        engine.shard_id(),
        vec![MempoolMessage::ValidatorMessage(proto::ValidatorMessage {
            on_chain_event: None,
            fname_transfer: Some(transfer.clone()),
        })],
        None,
    );

    validate_and_commit_state_change(engine, &state_change);

    // let proof = transfer.proof.as_ref().unwrap();
    // let name = String::from_utf8(proof.name.clone()).unwrap();
    // assert!(engine.trie_key_exists(trie_ctx(), &TrieKey::for_fname(proof.fid, &name)));
}

#[cfg(test)]
pub async fn register_fname(
    fid: u64,
    username: &String,
    timestamp: Option<u32>,
    owner: Option<Vec<u8>>,
    engine: &mut ShardEngine,
    network: proto::FarcasterNetwork,
    signer: alloy_signer_local::PrivateKeySigner,
) {
    use crate::core::validations::verification;

    let fname_transfer =
        username_factory::create_transfer(fid, username, timestamp, None, owner, signer.clone());

    assert!(verification::validate_fname_transfer(
        &fname_transfer,
        network,
        Some(signer.address())
    )
    .is_ok());

    let state_change = engine.propose_state_change(
        engine.shard_id(),
        vec![MempoolMessage::ValidatorMessage(proto::ValidatorMessage {
            on_chain_event: None,
            fname_transfer: Some(fname_transfer.clone()),
        })],
        None,
    );

    validate_and_commit_state_change(engine, &state_change);

    // Ensure the key exists in the trie as this can fail silently otherwise
    assert!(key_exists_in_trie(
        engine,
        &TrieKey::for_fname(fid, username)
    ));
}

pub fn default_signer() -> SigningKey {
    SigningKey::from_bytes(
        &SecretKey::from_hex("1000000000000000000000000000000000000000000000000000000000000000")
            .unwrap(),
    )
}

pub fn default_custody_address() -> Vec<u8> {
    "000000000000000000".to_string().encode_to_vec()
}

pub fn generate_signer() -> SigningKey {
    let mut secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret);
    SigningKey::from_bytes(&secret)
}

#[allow(dead_code)]
pub fn enable_logging() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .try_init();
}

#[allow(dead_code)]
pub fn assert_contains_message(
    container: &dyn MessagesContainer,
    expected_message: &proto::Message,
) {
    assert!(container
        .messages()
        .iter()
        .find(|m| bytes_compare(&m.hash, &expected_message.hash) == 0)
        .is_some());
}

#[allow(dead_code)]
pub fn assert_does_not_contain_message(
    container: &dyn MessagesContainer,
    expected_message: &proto::Message,
) {
    assert!(container
        .messages()
        .iter()
        .find(|m| bytes_compare(&m.hash, &expected_message.hash) == 0)
        .is_none());
}

#[allow(dead_code)]
pub fn assert_contains_all_messages(
    container: &dyn MessagesContainer,
    expected_messages: &[&proto::Message],
) {
    assert_eq!(container.messages().len(), expected_messages.len());
    for message in expected_messages {
        assert_contains_message(container.messages(), message);
    }
}

#[allow(dead_code)]
pub fn assert_messages_empty(messages: &dyn MessagesContainer) {
    assert_eq!(messages.messages().len(), 0);
}

#[allow(dead_code)]
pub trait MessagesContainer {
    fn messages(&self) -> &Vec<proto::Message>;
}

impl MessagesContainer for MessagesPage {
    fn messages(&self) -> &Vec<proto::Message> {
        &self.messages
    }
}

impl MessagesContainer for Result<MessagesPage, HubError> {
    fn messages(&self) -> &Vec<proto::Message> {
        assert!(self.is_ok());
        &self.as_ref().unwrap().messages
    }
}

impl MessagesContainer for Result<Response<MessagesResponse>, Status> {
    fn messages(&self) -> &Vec<proto::Message> {
        assert!(self.is_ok());
        &self.as_ref().unwrap().get_ref().messages
    }
}

impl MessagesContainer for Response<MessagesResponse> {
    fn messages(&self) -> &Vec<proto::Message> {
        &self.get_ref().messages
    }
}

impl MessagesContainer for Vec<proto::Message> {
    fn messages(&self) -> &Vec<proto::Message> {
        self
    }
}
