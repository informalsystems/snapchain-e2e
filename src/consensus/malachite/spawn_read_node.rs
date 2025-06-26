use informalsystems_malachitebft_config::ValueSyncConfig;
use informalsystems_malachitebft_engine::network::NetworkRef;
use informalsystems_malachitebft_sync::Metrics as SyncMetrics;
use std::collections::BTreeMap;
use tracing::Span;

use crate::consensus::consensus::{Config, SystemMessage};
use crate::consensus::malachite::network_connector::{
    MalachiteNetworkActorMsg, MalachiteNetworkEvent,
};
use crate::consensus::read_validator::{self, Engine};
use crate::consensus::validator::{StoredValidatorSet, StoredValidatorSets};
use crate::core::types::{ShardId, SnapchainValidatorContext};
use crate::network::gossip::GossipEvent;
use crate::proto::{self, Height};
use crate::utils::statsd_wrapper::StatsdClientWrapper;
use informalsystems_malachitebft_metrics::SharedRegistry;
use libp2p::PeerId;
use tokio::sync::mpsc;

use super::read_host::{ReadHost, ReadHostMsg, ReadHostRef, ReadHostState};
use super::read_sync::{ReadParams, ReadSync, ReadSyncRef};
use super::spawn::spawn_network_actor;

pub async fn spawn_read_host(
    shard_id: u32,
    statsd_client: StatsdClientWrapper,
    engine: Engine,
    system_tx: mpsc::Sender<SystemMessage>,
    config: Config,
) -> Result<ReadHostRef, ractor::SpawnErr> {
    let validator_set_config = config.get_validator_set_config(shard_id);
    let validator_sets = validator_set_config
        .iter()
        .map(|config| StoredValidatorSet::new(ShardId::new(shard_id), &config))
        .collect();
    let state = ReadHostState {
        validator: read_validator::ReadValidator {
            shard_id,
            engine,
            last_height: Height {
                shard_index: shard_id,
                block_number: 0,
            },
            max_num_buffered_blocks: 100,
            buffered_blocks: BTreeMap::new(),
            validator_sets: StoredValidatorSets::new(shard_id, validator_sets),
            statsd_client,
            system_tx: system_tx.clone(),
        },
        system_tx,
    };
    let actor_ref = ReadHost::spawn(state).await?;
    Ok(actor_ref)
}

pub async fn spawn_read_sync_actor(
    ctx: SnapchainValidatorContext,
    network: NetworkRef<SnapchainValidatorContext>,
    host: ReadHostRef,
    config: ValueSyncConfig,
    registry: &SharedRegistry,
    span: Span,
) -> Result<ReadSyncRef, ractor::SpawnErr> {
    let params = ReadParams {
        status_update_interval: config.status_update_interval,
        request_timeout: config.request_timeout,
    };

    let metrics = SyncMetrics::register(registry);

    let actor_ref = ReadSync::spawn(ctx, network, host, params, metrics, span).await?;

    Ok(actor_ref)
}

#[derive(Clone)]
pub struct MalachiteReadNodeActors {
    pub network_actor: NetworkRef<SnapchainValidatorContext>,
    pub host_actor: ReadHostRef,
    pub sync_actor: ReadSyncRef,
}

impl MalachiteReadNodeActors {
    pub async fn create_and_start(
        ctx: SnapchainValidatorContext,
        engine: Engine,
        local_peer_id: PeerId,
        gossip_tx: mpsc::Sender<GossipEvent<SnapchainValidatorContext>>,
        system_tx: mpsc::Sender<SystemMessage>,
        registry: &SharedRegistry,
        shard_id: u32,
        statsd_client: StatsdClientWrapper,
        config: Config,
    ) -> Result<Self, ractor::SpawnErr> {
        let name = if shard_id == 0 {
            format!("Block")
        } else {
            format!("Shard {}", shard_id)
        };
        let span = tracing::info_span!("node", name = %name);

        let sync_config = ValueSyncConfig {
            request_timeout: config.sync_request_timeout,
            ..ValueSyncConfig::default()
        };
        let network_actor = spawn_network_actor(gossip_tx.clone(), local_peer_id).await?;
        let host_actor =
            spawn_read_host(shard_id, statsd_client, engine, system_tx, config).await?;
        let sync_actor = spawn_read_sync_actor(
            ctx.clone(),
            network_actor.clone(),
            host_actor.clone(),
            sync_config,
            registry,
            span.clone(),
        )
        .await?;

        host_actor
            .cast(ReadHostMsg::Started {
                sync: sync_actor.clone(),
            })
            .unwrap();

        Ok(Self {
            network_actor,
            host_actor,
            sync_actor,
        })
    }

    pub fn cast_decided_value(
        &self,
        value: proto::DecidedValue,
    ) -> Result<(), ractor::MessagingErr<ReadHostMsg>> {
        self.host_actor.cast(ReadHostMsg::ProcessDecidedValue {
            value,
            sync: self.sync_actor.clone(),
        })
    }

    pub fn cast_network_event(
        &self,
        event: MalachiteNetworkEvent,
    ) -> Result<(), ractor::MessagingErr<MalachiteNetworkActorMsg>> {
        self.network_actor
            .cast(MalachiteNetworkActorMsg::NewEvent(event))
    }

    pub fn stop(&self) {
        self.host_actor.stop(None);
        self.network_actor.stop(None);
        self.sync_actor.stop(None);
    }
}
