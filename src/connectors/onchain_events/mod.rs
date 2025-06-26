use crate::cfg::Config as AppConfig;
use crate::proto::TierPurchaseBody;
use crate::storage::store::node_local_state;
use alloy_primitives::U256;
use alloy_primitives::{address, ruint::FromUintError, Address, FixedBytes};
use alloy_provider::{Provider, ProviderBuilder, RootProvider};
use alloy_rpc_types::{Filter, Log};
use alloy_sol_types::{sol, SolEvent};
use alloy_transport_http::{Client, Http};
use async_trait::async_trait;
use foundry_common::ens::EnsResolver::EnsResolverInstance;
use foundry_common::ens::{namehash, EnsError, EnsRegistry};
use futures_util::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, warn};

use crate::core::error::HubError;
use crate::mempool::mempool::{MempoolRequest, MempoolSource};
use crate::{
    core::validations::{
        self,
        verification::{validate_verification_contract_signature, VerificationAddressClaim},
    },
    proto::{
        on_chain_event, IdRegisterEventBody, IdRegisterEventType, OnChainEvent, OnChainEventType,
        SignerEventBody, SignerEventType, SignerMigratedEventBody, StorageRentEventBody,
        ValidatorMessage, VerificationAddAddressBody,
    },
    storage::store::{engine::MempoolMessage, node_local_state::LocalStateStore},
    utils::statsd_wrapper::StatsdClientWrapper,
};

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    StorageRegistryAbi,
    "src/connectors/onchain_events/storage_registry_abi.json"
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    IdRegistryAbi,
    "src/connectors/onchain_events/id_registry_abi.json"
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    KeyRegistryAbi,
    "src/connectors/onchain_events/key_registry_abi.json"
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    TierRegistryAbi,
    "src/connectors/onchain_events/tier_registry_abi.json"
);

// Note these are the registry addresses, not the resolver addresses. We look up the resolver from the registry.
static ETH_L1_ENS_REGISTRY: Address = address!("00000000000C2E074eC69A0dFb2997BA6C7d2e1e");
static BASE_MAINNET_ENS_REGISTRY: Address = address!("0xB94704422c2a1E396835A571837Aa5AE53285a95");

// For reference, in case it needs to be specified manually
const OP_MAINNET_FIRST_BLOCK: u64 = 108864739;
static OP_MAINNET_CHAIN_ID: u32 = 10; // OP mainnet
const BASE_MAINNET_FIRST_BLOCK: u64 = 31180908;
static BASE_MAINNET_CHAIN_ID: u32 = 8453; // Base mainnet
const RENT_EXPIRY_IN_SECONDS: u64 = 365 * 24 * 60 * 60; // One year

const RETRY_TIMEOUT_SECONDS: u64 = 10;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub rpc_url: String,
    pub start_block_number: Option<u64>,
    pub stop_block_number: Option<u64>,
    pub override_tier_registry_address: Option<String>, // For testing
}

impl Default for Config {
    fn default() -> Config {
        return Config {
            rpc_url: String::new(),
            start_block_number: None,
            stop_block_number: None,
            override_tier_registry_address: None,
        };
    }
}

#[derive(Clone)]
pub enum OnchainEventsRequest {
    RetryFid(u64),
    RetryBlockRange {
        start_block_number: u64,
        stop_block_number: u64,
    },
}

#[derive(Error, Debug)]
pub enum SubscribeError {
    #[error(transparent)]
    UnableToSubscribe(#[from] alloy_transport::TransportError),

    #[error(transparent)]
    UnableToParseUrl(#[from] url::ParseError),

    #[error(transparent)]
    UnableToParseLog(#[from] alloy_sol_types::Error),

    #[error(transparent)]
    UnableToConvertToU64(#[from] FromUintError<u64>),

    #[error(transparent)]
    UnableToConvertToU32(#[from] FromUintError<u32>),

    #[error(transparent)]
    UnableToConvertToI32(#[from] FromUintError<i32>),

    #[error("Empty rpc url")]
    EmptyRpcUrl,

    #[error("Log missing block hash")]
    LogMissingBlockHash,

    #[error("Log missing log index")]
    LogMissingLogIndex,

    #[error("Log missing block number")]
    LogMissingBlockNumber,

    #[error("Log missing tx index")]
    LogMissingTxIndex,

    #[error("Log missing tx hash")]
    LogMissingTransactionHash,

    #[error("Unable to find block by hash")]
    UnableToFindBlockByHash,
}

#[async_trait]
pub trait ChainAPI: Send + Sync {
    async fn resolve_ens_name(&self, name: String) -> Result<Address, EnsError>;
    async fn verify_contract_signature(
        &self,
        claim: VerificationAddressClaim,
        body: &VerificationAddAddressBody,
    ) -> Result<(), validations::error::ValidationError>;
}

#[derive(Eq, Hash, PartialEq, Debug)]
pub enum Chain {
    EthMainnet,
    BaseMainnet,
}

pub struct ChainClients {
    pub chain_api_map: HashMap<Chain, Box<dyn ChainAPI>>,
}

impl ChainClients {
    pub fn new(app_config: &AppConfig) -> Self {
        let mut chain_api_map = HashMap::new();
        if !app_config.l1_rpc_url.is_empty() {
            let client: Box<dyn ChainAPI> = Box::new(
                RealL1Client::new(app_config.l1_rpc_url.clone(), ETH_L1_ENS_REGISTRY).unwrap(),
            );
            chain_api_map.insert(Chain::EthMainnet, client);
        }
        if !app_config.base_onchain_events.rpc_url.is_empty() {
            let client: Box<dyn ChainAPI> = Box::new(
                RealL1Client::new(
                    app_config.base_onchain_events.rpc_url.clone(),
                    BASE_MAINNET_ENS_REGISTRY,
                )
                .unwrap(),
            );
            chain_api_map.insert(Chain::BaseMainnet, client);
        }

        ChainClients { chain_api_map }
    }

    pub fn for_chain(&self, chain: Chain) -> Result<&Box<dyn ChainAPI>, HubError> {
        match self.chain_api_map.get(&chain) {
            Some(client) => Ok(client),
            None => Err(HubError::invalid_internal_state(
                format!("No client configured for chain: {:?}", chain).as_str(),
            )),
        }
    }
}

pub struct RealL1Client {
    provider: RootProvider<Http<Client>>,
    ens_resolver_address: Address,
}

impl RealL1Client {
    pub fn new(
        rpc_url: String,
        ens_resolver_address: Address,
    ) -> Result<RealL1Client, SubscribeError> {
        if rpc_url.is_empty() {
            return Err(SubscribeError::EmptyRpcUrl);
        }
        let url = rpc_url.parse()?;
        let provider = ProviderBuilder::new().on_http(url);
        Ok(RealL1Client {
            provider,
            ens_resolver_address,
        })
    }
}

#[async_trait]
impl ChainAPI for RealL1Client {
    async fn resolve_ens_name(&self, name: String) -> Result<Address, EnsError> {
        // Copied from foundry_common::ens so we can support both ETH and Base mainnet
        let node = namehash(name.as_str());

        let registry = EnsRegistry::new(self.ens_resolver_address, self.provider.clone());
        let address = registry
            .resolver(node)
            .call()
            .await
            .map_err(EnsError::Resolver)?
            ._0;
        if address == Address::ZERO {
            return Err(EnsError::ResolverNotFound(name.to_string()));
        }
        let resolver = EnsResolverInstance::new(address, self.provider.clone());
        let addr = resolver
            .addr(node)
            .call()
            .await
            .map_err(EnsError::Resolve)
            .inspect_err(|e| {
                warn!("Failed to resolve ens name {name}: {}", e);
            })?
            ._0;
        Ok(addr)
    }

    async fn verify_contract_signature(
        &self,
        claim: VerificationAddressClaim,
        body: &VerificationAddAddressBody,
    ) -> Result<(), validations::error::ValidationError> {
        validate_verification_contract_signature(&self.provider, claim, body).await
    }
}

#[derive(Clone)]
pub enum ContractKind {
    TierRegistry,
    StorageRegistry,
    KeyRegistry,
    IdRegistry,
}
#[derive(Clone)]
pub struct Contract {
    address: Address,
    kind: ContractKind,
}

impl Contract {
    pub fn storage_registry() -> Self {
        Contract {
            address: address!("00000000fcce7f938e7ae6d3c335bd6a1a7c593d"),
            kind: ContractKind::StorageRegistry,
        }
    }

    pub fn key_registry() -> Self {
        Contract {
            address: address!("00000000Fc1237824fb747aBDE0FF18990E59b7e"),
            kind: ContractKind::KeyRegistry,
        }
    }

    pub fn id_registry() -> Self {
        Contract {
            address: address!("00000000Fc6c5F01Fc30151999387Bb99A9f489b"),
            kind: ContractKind::IdRegistry,
        }
    }

    pub fn tier_registry() -> Self {
        Contract {
            address: address!("0x00000000fc84484d585C3cF48d213424DFDE43FD"),
            kind: ContractKind::TierRegistry,
        }
    }

    pub fn event_kind(&self) -> &str {
        match self.kind {
            ContractKind::TierRegistry => "tier",
            ContractKind::StorageRegistry => "storage",
            ContractKind::KeyRegistry => "key",
            ContractKind::IdRegistry => "id",
        }
    }

    pub fn retry_filters(&self, fid: u64, start_block: u64) -> Vec<Filter> {
        match self.kind {
            ContractKind::TierRegistry => {
                vec![Filter::new()
                    .address(vec![self.address])
                    .from_block(start_block)
                    .events(vec!["PurchasedTier(uint256,uint256,uint256,address)"])
                    .topic1(U256::from(fid))]
            }
            ContractKind::StorageRegistry => {
                vec![Filter::new()
                    .address(vec![self.address])
                    .from_block(start_block)
                    .events(vec!["Rent(address,uint256,uint256)"])
                    .topic2(U256::from(fid))]
            }
            ContractKind::KeyRegistry => {
                vec![Filter::new()
                    .address(vec![self.address])
                    .from_block(start_block)
                    .events(vec![
                        "Add(uint256,uint32,bytes,bytes,uint8,bytes)",
                        "Remove(uint256,bytes,bytes)",
                    ])
                    .topic1(U256::from(fid))]
            }
            ContractKind::IdRegistry => {
                vec![
                    Filter::new()
                        .address(vec![self.address])
                        .from_block(start_block)
                        .events(vec!["Register(address,uint256,address)"])
                        .topic2(U256::from(fid)),
                    Filter::new()
                        .address(vec![self.address])
                        .from_block(start_block)
                        .events(vec!["Transfer(address,address,uint256)"])
                        .topic3(U256::from(fid)),
                ]
            }
        }
    }
}

pub struct Subscriber {
    provider: RootProvider<Http<Client>>,
    mempool_tx: mpsc::Sender<MempoolRequest>,
    start_block_number: Option<u64>,
    stop_block_number: Option<u64>,
    statsd_client: StatsdClientWrapper,
    local_state_store: LocalStateStore,
    onchain_events_request_rx: broadcast::Receiver<OnchainEventsRequest>,
    chain: node_local_state::Chain,
    override_tier_registry_address: Option<String>,
}

// TODO(aditi): Wait for 1 confirmation before "committing" an onchain event.
impl Subscriber {
    pub fn new(
        config: &Config,
        chain: node_local_state::Chain,
        mempool_tx: mpsc::Sender<MempoolRequest>,
        statsd_client: StatsdClientWrapper,
        local_state_store: LocalStateStore,
        onchain_events_request_rx: broadcast::Receiver<OnchainEventsRequest>,
    ) -> Result<Subscriber, SubscribeError> {
        if config.rpc_url.is_empty() {
            return Err(SubscribeError::EmptyRpcUrl);
        }
        let url = config.rpc_url.parse()?;
        let provider = ProviderBuilder::new().on_http(url);
        Ok(Subscriber {
            local_state_store,
            provider,
            mempool_tx,
            start_block_number: config
                .start_block_number
                .map(|start_block| start_block.max(Self::first_block(chain))),
            stop_block_number: config.stop_block_number,
            statsd_client,
            onchain_events_request_rx,
            chain,
            override_tier_registry_address: config.override_tier_registry_address.clone(),
        })
    }

    fn contracts(&self) -> Vec<Contract> {
        match self.chain {
            node_local_state::Chain::Optimism => vec![
                Contract::storage_registry(),
                Contract::key_registry(),
                Contract::id_registry(),
            ],
            node_local_state::Chain::Base => vec![match &self.override_tier_registry_address {
                None => Contract::tier_registry(),
                Some(tier_registry_address) => Contract {
                    address: Address::from_str(&tier_registry_address).unwrap(),
                    kind: ContractKind::TierRegistry,
                },
            }],
        }
    }

    fn first_block(chain: node_local_state::Chain) -> u64 {
        match chain {
            node_local_state::Chain::Optimism => OP_MAINNET_FIRST_BLOCK,
            node_local_state::Chain::Base => BASE_MAINNET_FIRST_BLOCK,
        }
    }

    fn chain_id(chain: node_local_state::Chain) -> u32 {
        match chain {
            node_local_state::Chain::Optimism => OP_MAINNET_CHAIN_ID,
            node_local_state::Chain::Base => BASE_MAINNET_CHAIN_ID,
        }
    }

    fn count(&self, key: &str, value: i64) {
        self.statsd_client
            .count(format!("onchain_events.{}", key).as_str(), value);
    }

    fn gauge(&self, key: &str, value: u64) {
        self.statsd_client
            .gauge(format!("onchain_events.{}", key).as_str(), value);
    }

    async fn add_onchain_event(
        &mut self,
        fid: u64,
        block_number: u32,
        block_hash: FixedBytes<32>,
        block_timestamp: u64,
        log_index: u32,
        tx_index: u32,
        transaction_hash: FixedBytes<32>,
        event_type: OnChainEventType,
        event_body: on_chain_event::Body,
    ) {
        let event = OnChainEvent {
            fid,
            block_number,
            block_hash: block_hash.to_vec(),
            block_timestamp,
            log_index,
            tx_index,
            r#type: event_type as i32,
            chain_id: Self::chain_id(self.chain),
            version: 0,
            body: Some(event_body),
            transaction_hash: transaction_hash.to_vec(),
        };
        info!(
            fid,
            event_type = event_type.as_str_name(),
            block_number = event.block_number,
            block_timestamp = event.block_timestamp,
            tx_hash = hex::encode(&event.transaction_hash),
            log_index = event.log_index,
            chain = self.chain.to_string(),
            "Processed onchain event"
        );
        match event_type {
            OnChainEventType::EventTypeNone => {}
            OnChainEventType::EventTypeSigner => {
                self.count("num_signer_events", 1);
            }
            OnChainEventType::EventTypeSignerMigrated => {
                self.count("num_signer_migrated_events", 1);
            }
            OnChainEventType::EventTypeIdRegister => {
                self.count("num_id_register_events", 1);
            }
            OnChainEventType::EventTypeStorageRent => {
                self.count("num_storage_events", 1);
            }
            OnChainEventType::EventTypeTierPurchase => {
                self.count("num_tier_purchase_events", 1);
            }
        };
        match &event.body {
            Some(on_chain_event::Body::IdRegisterEventBody(id_register_event_body)) => {
                if id_register_event_body.event_type() == IdRegisterEventType::Register {
                    self.gauge("latest_fid_registered", fid);
                }
            }
            _ => {}
        }
        self.gauge(
            &format!("latest_block_number_on_{}", self.chain.to_string()),
            block_number as u64,
        );
        if let Err(err) = self
            .mempool_tx
            .send(MempoolRequest::AddMessage(
                MempoolMessage::ValidatorMessage(ValidatorMessage {
                    on_chain_event: Some(event.clone()),
                    fname_transfer: None,
                }),
                MempoolSource::Local,
                None,
            ))
            .await
        {
            error!(
                block_number = event.block_number,
                tx_hash = hex::encode(&event.transaction_hash),
                log_index = event.log_index,
                err = err.to_string(),
                chain = self.chain.to_string(),
                "Unable to send onchain event to mempool"
            )
        }
    }

    fn record_block_number(&self, block_number: u64) {
        let latest_block_in_db = self.latest_block_in_db();
        if block_number as u64 > latest_block_in_db {
            match self
                .local_state_store
                .set_latest_block_number(self.chain.clone(), block_number)
            {
                Err(err) => {
                    error!(
                        block_number,
                        err = err.to_string(),
                        chain = self.chain.to_string(),
                        "Unable to store last block number",
                    );
                }
                _ => {}
            }
        };
    }

    async fn get_block_timestamp(&self, block_hash: FixedBytes<32>) -> Result<u64, SubscribeError> {
        let mut retry_count = 0;
        loop {
            match self
                .provider
                .get_block_by_hash(block_hash, alloy_rpc_types::BlockTransactionsKind::Hashes)
                .await
            {
                Ok(Some(block)) => {
                    return Ok(block.header.timestamp);
                }
                Ok(None) => {
                    return Err(SubscribeError::UnableToFindBlockByHash);
                }
                Err(err) => {
                    retry_count += 1;

                    if retry_count > 5 {
                        return Err(err.into());
                    }

                    error!(
                        chain = self.chain.to_string(),
                        "Error getting block timestamp for hash {}: {}. Retry {} in {} seconds",
                        hex::encode(block_hash),
                        err,
                        retry_count,
                        RETRY_TIMEOUT_SECONDS
                    );

                    tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_TIMEOUT_SECONDS))
                        .await;
                }
            }
        }
    }

    async fn process_log(&mut self, event: &Log) -> Result<(), SubscribeError> {
        let block_hash = event
            .block_hash
            .ok_or(SubscribeError::LogMissingBlockHash)?;
        let log_index = event.log_index.ok_or(SubscribeError::LogMissingLogIndex)?;
        let block_number = event
            .block_number
            .ok_or(SubscribeError::LogMissingBlockNumber)?;
        let tx_index = event
            .transaction_index
            .ok_or(SubscribeError::LogMissingTxIndex)?;
        let transaction_hash = event
            .transaction_hash
            .ok_or(SubscribeError::LogMissingTransactionHash)?;
        // TODO(aditi): Cache these queries for timestamp to optimize rpc calls.
        // [block_timestamp] exists on [Log], however it's never populated in practice.
        let block_timestamp = self.get_block_timestamp(block_hash).await?;
        let add_event = |fid, event_type, event_body| async move {
            self.add_onchain_event(
                fid,
                block_number as u32,
                block_hash,
                block_timestamp,
                log_index as u32,
                tx_index as u32,
                transaction_hash,
                event_type,
                event_body,
            )
            .await;
        };
        match event.topic0() {
            Some(&StorageRegistryAbi::Rent::SIGNATURE_HASH) => {
                let StorageRegistryAbi::Rent { payer, fid, units } = event.log_decode()?.inner.data;
                let fid = fid.try_into()?;
                add_event(
                    fid,
                    OnChainEventType::EventTypeStorageRent,
                    on_chain_event::Body::StorageRentEventBody(StorageRentEventBody {
                        payer: payer.to_vec(),
                        units: units.try_into()?,
                        expiry: (block_timestamp + RENT_EXPIRY_IN_SECONDS) as u32,
                    }),
                )
                .await;
                Ok(())
            }
            Some(&IdRegistryAbi::Register::SIGNATURE_HASH) => {
                let IdRegistryAbi::Register { to, id, recovery } = event.log_decode()?.inner.data;
                let fid = id.try_into()?;
                add_event(
                    fid,
                    OnChainEventType::EventTypeIdRegister,
                    on_chain_event::Body::IdRegisterEventBody(IdRegisterEventBody {
                        event_type: IdRegisterEventType::Register as i32,
                        to: to.to_vec(),
                        recovery_address: recovery.to_vec(),
                        from: vec![],
                    }),
                )
                .await;
                Ok(())
            }
            Some(&IdRegistryAbi::Transfer::SIGNATURE_HASH) => {
                let IdRegistryAbi::Transfer { from, to, id } = event.log_decode()?.inner.data;
                let fid = id.try_into()?;
                add_event(
                    fid,
                    OnChainEventType::EventTypeIdRegister,
                    on_chain_event::Body::IdRegisterEventBody(IdRegisterEventBody {
                        event_type: IdRegisterEventType::Transfer as i32,
                        to: to.to_vec(),
                        from: from.to_vec(),
                        recovery_address: vec![],
                    }),
                )
                .await;
                Ok(())
            }
            Some(&IdRegistryAbi::ChangeRecoveryAddress::SIGNATURE_HASH) => {
                let IdRegistryAbi::ChangeRecoveryAddress { id, recovery } =
                    event.log_decode()?.inner.data;
                let fid = id.try_into()?;
                add_event(
                    fid,
                    OnChainEventType::EventTypeIdRegister,
                    on_chain_event::Body::IdRegisterEventBody(IdRegisterEventBody {
                        event_type: IdRegisterEventType::ChangeRecovery as i32,
                        to: vec![],
                        from: vec![],
                        recovery_address: recovery.to_vec(),
                    }),
                )
                .await;
                Ok(())
            }
            Some(&KeyRegistryAbi::Add::SIGNATURE_HASH) => {
                let KeyRegistryAbi::Add {
                    fid,
                    key: _,
                    keytype,
                    keyBytes,
                    metadatatype,
                    metadata,
                } = event.log_decode()?.inner.data;
                let fid = fid.try_into()?;
                add_event(
                    fid,
                    OnChainEventType::EventTypeSigner,
                    on_chain_event::Body::SignerEventBody(SignerEventBody {
                        key: keyBytes.to_vec(),
                        key_type: keytype,
                        event_type: SignerEventType::Add as i32,
                        metadata: metadata.to_vec(),
                        metadata_type: metadatatype as u32,
                    }),
                )
                .await;
                Ok(())
            }
            Some(&KeyRegistryAbi::Remove::SIGNATURE_HASH) => {
                let KeyRegistryAbi::Remove {
                    fid,
                    key: _,
                    keyBytes,
                } = event.log_decode()?.inner.data;
                let fid = fid.try_into()?;
                add_event(
                    fid,
                    OnChainEventType::EventTypeSigner,
                    on_chain_event::Body::SignerEventBody(SignerEventBody {
                        key: keyBytes.to_vec(),
                        key_type: 0,
                        event_type: SignerEventType::Remove as i32,
                        metadata: vec![],
                        metadata_type: 0,
                    }),
                )
                .await;
                Ok(())
            }
            Some(&KeyRegistryAbi::AdminReset::SIGNATURE_HASH) => {
                let KeyRegistryAbi::AdminReset {
                    fid,
                    key: _,
                    keyBytes,
                } = event.log_decode()?.inner.data;
                let fid = fid.try_into()?;
                add_event(
                    fid,
                    OnChainEventType::EventTypeSigner,
                    on_chain_event::Body::SignerEventBody(SignerEventBody {
                        key: keyBytes.to_vec(),
                        key_type: 0,
                        event_type: SignerEventType::AdminReset as i32,
                        metadata: vec![],
                        metadata_type: 0,
                    }),
                )
                .await;
                Ok(())
            }
            Some(&KeyRegistryAbi::Migrated::SIGNATURE_HASH) => {
                let KeyRegistryAbi::Migrated { keysMigratedAt } = event.log_decode()?.inner.data;
                let migrated_at = keysMigratedAt.try_into()?;
                add_event(
                    0,
                    OnChainEventType::EventTypeSignerMigrated,
                    on_chain_event::Body::SignerMigratedEventBody(SignerMigratedEventBody {
                        migrated_at,
                    }),
                )
                .await;
                Ok(())
            }
            Some(&TierRegistryAbi::PurchasedTier::SIGNATURE_HASH) => {
                let TierRegistryAbi::PurchasedTier {
                    fid,
                    tier,
                    forDays,
                    payer,
                } = event.log_decode()?.inner.data;
                add_event(
                    fid.try_into()?,
                    OnChainEventType::EventTypeTierPurchase,
                    on_chain_event::Body::TierPurchaseEventBody(TierPurchaseBody {
                        tier_type: tier.try_into()?,
                        for_days: forDays.try_into()?,
                        payer: payer.to_vec(),
                    }),
                )
                .await;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn get_logs(&mut self, filter: &Filter, event_kind: &str) -> Result<(), SubscribeError> {
        let events = self.provider.get_logs(filter).await?;
        for event in events {
            let result = self.process_log(&event).await;
            match result {
                Err(err) => {
                    error!(
                        chain = self.chain.to_string(),
                        event_kind,
                        "Error processing onchain event. Error: {:#?}. Event: {:#?}",
                        err,
                        event,
                    )
                }
                Ok(()) => {}
            }
        }
        Ok(())
    }

    async fn get_logs_with_retry(
        &mut self,
        filter: Filter,
        event_kind: &str,
    ) -> Result<(), SubscribeError> {
        let mut retry_count = 0;
        loop {
            match self.get_logs(&filter, event_kind).await {
                Ok(_) => return Ok(()),
                Err(err) => {
                    retry_count += 1;

                    if retry_count > 5 {
                        return Err(err);
                    }

                    error!(
                        chain = self.chain.to_string(),
                        "Error getting logs for {} event kind(s): {}. Retry {} in {} seconds",
                        event_kind,
                        err,
                        retry_count,
                        RETRY_TIMEOUT_SECONDS
                    );

                    tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_TIMEOUT_SECONDS))
                        .await;
                }
            }
        }
    }

    pub async fn sync_historical_events(
        &mut self,
        initial_start_block: u64,
        final_stop_block: u64,
    ) -> Result<(), SubscribeError> {
        info!(
            start_block_number = initial_start_block,
            stop_block_number = final_stop_block,
            chain = self.chain.to_string(),
            "Starting historical sync"
        );
        let batch_size = 1000;
        let mut start_block = initial_start_block;
        loop {
            let stop_block = final_stop_block.min(start_block + batch_size);

            for contract in self.contracts() {
                let filter = Filter::new()
                    .address(contract.address)
                    .from_block(start_block)
                    .to_block(stop_block);
                self.get_logs_with_retry(filter, contract.event_kind())
                    .await?;
            }

            self.record_block_number(stop_block);
            start_block += batch_size;

            if start_block > final_stop_block {
                info!(
                    start_block,
                    stop_block = final_stop_block,
                    chain = self.chain.to_string(),
                    "Stopping historical sync"
                );
                return Ok(());
            }
        }
    }

    fn latest_block_in_db(&self) -> u64 {
        match self
            .local_state_store
            .get_latest_block_number(self.chain.clone())
        {
            Ok(number) => number.unwrap_or(0),
            Err(err) => {
                error!(
                    err = err.to_string(),
                    chain = self.chain.to_string(),
                    "Unable to retrieve last block number",
                );
                0
            }
        }
    }

    async fn latest_block_on_chain(&mut self) -> Result<u64, SubscribeError> {
        let mut retry_count = 0;
        loop {
            match self
                .provider
                .get_block_by_number(
                    alloy_rpc_types::BlockNumberOrTag::Latest,
                    alloy_rpc_types::BlockTransactionsKind::Hashes,
                )
                .await
            {
                Ok(block) => {
                    return Ok(block
                        .ok_or(SubscribeError::LogMissingBlockNumber)?
                        .header
                        .number);
                }
                Err(err) => {
                    retry_count += 1;
                    if retry_count > 5 {
                        return Err(err.into());
                    }

                    error!(
                        chain = self.chain.to_string(),
                        "Error getting latest block on chain: {}. Retry {} in {} seconds",
                        err,
                        retry_count,
                        RETRY_TIMEOUT_SECONDS
                    );

                    tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_TIMEOUT_SECONDS))
                        .await;
                }
            }
        }
    }

    async fn sync_live_events(&mut self, start_block_number: u64) -> Result<(), SubscribeError> {
        info!(
            start_block_number,
            chain = self.chain.to_string(),
            "Starting live sync"
        );
        let contract_addresses: Vec<Address> = self
            .contracts()
            .iter()
            .map(|contract| contract.address)
            .collect();
        let filter = Filter::new()
            .address(contract_addresses)
            .from_block(start_block_number);

        let filter = match self.stop_block_number {
            None => filter,
            Some(stop_block) => filter.to_block(stop_block),
        };

        let subscription = self.provider.watch_logs(&filter).await?;
        let mut stream = subscription.into_stream();
        loop {
            tokio::select! {
                 biased;

                 request = self.onchain_events_request_rx.recv() => {
                    match request {
                        Err(_) => {
                            // Ignore, this can happen if we don't run an admin server
                        }, Ok(request) => {
                            match request {
                                OnchainEventsRequest::RetryFid(retry_fid) =>  {
                                    if let Err(err) = self.retry_fid(retry_fid).await {
                                        error!(fid = retry_fid, chain = self.chain.to_string(),
                                             "Unable to retry fid: {}", err.to_string())
                                    }
                                },
                                OnchainEventsRequest::RetryBlockRange{start_block_number, stop_block_number} => {
                                    if let Err(err) = self.retry_block_range(start_block_number, stop_block_number).await {
                                        error!(start_block_number, stop_block_number, chain = self.chain.to_string(),
                                            "Unable to retry block range: {}", err.to_string())
                                    }


                                }
                            }
                        }
                    }
                 }
                 events = stream.next() => {
                     match events {
                         None => {
                            // We want to trigger a retry here
                             break;
                         },
                         Some(events) => {
                             for event in events {
                                 let result = self.process_log(&event).await;
                                 match result {
                                     Err(err) => {
                                         error!(
                                             "Error processing onchain event. Error: {:#?}. Event: {:#?}",
                                             err, event,
                                         )
                                     }
                                     Ok(()) => match event.block_number {
                                         None => {}
                                         Some(block_number) => {
                                             self.record_block_number(block_number);
                                         }
                                     },
                                 }
                             }
                         }
                     }
                 }
            }
        }
        Ok(())
    }

    pub async fn retry_fid(&mut self, fid: u64) -> Result<(), SubscribeError> {
        info!(
            fid,
            chain = self.chain.to_string(),
            "Retrying onchain events for fid"
        );
        for contract in self.contracts() {
            for retry_filter in contract.retry_filters(fid, Self::first_block(self.chain)) {
                self.get_logs_with_retry(retry_filter, contract.event_kind())
                    .await?;
            }
        }

        Ok(())
    }

    pub async fn retry_block_range(
        &mut self,
        start_block_number: u64,
        stop_block_number: u64,
    ) -> Result<(), SubscribeError> {
        info!(
            start_block_number,
            stop_block_number,
            chain = self.chain.to_string(),
            "Retrying onchain events in range"
        );
        let filter = Filter::new()
            .address(
                self.contracts()
                    .iter()
                    .map(|contract| contract.address)
                    .collect::<Vec<Address>>(),
            )
            .from_block(start_block_number)
            .to_block(stop_block_number);
        self.get_logs_with_retry(filter, "all").await?;
        Ok(())
    }

    pub async fn run(&mut self) -> Result<(), SubscribeError> {
        let latest_block_on_chain = self.latest_block_on_chain().await?;
        let latest_block_in_db = self.latest_block_in_db();
        info!(
            start_block_number = self.start_block_number,
            stop_block_numer = self.stop_block_number,
            latest_block_on_chain,
            latest_block_in_db,
            chain = self.chain.to_string(),
            "Starting l2 events subscription"
        );
        let live_sync_block;
        match self.start_block_number {
            None => {
                // By default, start from the first block or the latest block in the db. Whichever is higher
                live_sync_block = Some(Self::first_block(self.chain).max(latest_block_in_db));
            }
            Some(start_block_number) => {
                let historical_sync_start_block = latest_block_in_db.max(start_block_number);
                let historical_sync_stop_block = latest_block_on_chain
                    .min(self.stop_block_number.unwrap_or(latest_block_on_chain));

                // If we have a specific start block, sync historical events first
                self.sync_historical_events(
                    historical_sync_start_block,
                    historical_sync_stop_block,
                )
                .await?;

                live_sync_block = match self.stop_block_number {
                    // No specificed stop block, so live sync should resume from where historical sync ended
                    None => Some(historical_sync_stop_block),
                    Some(stop_block) => {
                        // stop block is in the future, so start live sync
                        if stop_block > historical_sync_stop_block {
                            Some(historical_sync_stop_block)
                        } else {
                            // stop block is in the past, so no need to live sync
                            None
                        }
                    }
                };
            }
        }

        if live_sync_block.is_none() {
            info!(
                chain = self.chain.to_string(),
                "Historical sync complete. Not subscribing to live events"
            );
            return Ok(());
        }

        loop {
            match self.sync_live_events(live_sync_block.unwrap()).await {
                Err(e) => {
                    error!(
                        chain = self.chain.to_string(),
                        "Live sync ended with error: {e}. Retrying in 10 seconds",
                    );
                }
                _ => {
                    error!(
                        chain = self.chain.to_string(),
                        "Live sync ended unexpectedly. Retrying in 10 seconds",
                    );
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_TIMEOUT_SECONDS)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::connectors::onchain_events;

    use super::*;

    #[tokio::test]
    #[ignore = "Requires a valid Alchemy API key"]
    async fn test_chain_clients() {
        // Test with a valid API key for Alchemy
        let api_key = "<KEY>";
        let app_config = AppConfig {
            l1_rpc_url: format!("https://eth-mainnet.g.alchemy.com/v2/{}", api_key).to_string(),
            base_onchain_events: onchain_events::Config {
                rpc_url: format!("https://base-mainnet.g.alchemy.com/v2/{}", api_key).to_string(),
                start_block_number: None,
                stop_block_number: None,
                override_tier_registry_address: None,
            },
            ..Default::default()
        };
        let chain_clients = ChainClients::new(&app_config);
        assert!(chain_clients.for_chain(Chain::EthMainnet).is_ok());
        assert!(chain_clients.for_chain(Chain::BaseMainnet).is_ok());

        let address = chain_clients
            .for_chain(Chain::EthMainnet)
            .unwrap()
            .resolve_ens_name("vitalik.eth".to_string())
            .await
            .unwrap();
        assert_eq!(
            address,
            address!("0xD8dA6BF26964aF9D7eEd9e03E53415D37aA96045")
        );

        let address = chain_clients
            .for_chain(Chain::BaseMainnet)
            .unwrap()
            .resolve_ens_name("jesse.base.eth".to_string())
            .await
            .unwrap();
        assert_eq!(
            address,
            address!("0x849151d7D0bF1F34b70d5caD5149D28CC2308bf1")
        );
    }
}
