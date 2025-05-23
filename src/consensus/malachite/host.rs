//! Implementation of a host actor for bridiging consensus and the application via a set of channels.

use crate::consensus::validator::{ProposalSource, ShardValidator};
use crate::core::types::SnapchainValidatorContext;
use crate::network::gossip::GossipEvent;
use crate::proto::{self, decided_value, full_proposal, Block, Commits, FullProposal, ShardChunk};
use crate::utils::statsd_wrapper::StatsdClientWrapper;
use bytes::Bytes;
use informalsystems_malachitebft_engine::consensus::ConsensusMsg;
use informalsystems_malachitebft_engine::host::{HostMsg, LocallyProposedValue};
use informalsystems_malachitebft_engine::network::{NetworkMsg, NetworkRef};
use informalsystems_malachitebft_engine::util::streaming::{
    StreamContent, StreamId, StreamMessage,
};
use informalsystems_malachitebft_sync::RawDecidedValue;
use prost::Message;
use ractor::{async_trait, Actor, ActorProcessingErr, ActorRef, SpawnErr};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Actor for bridging consensus and the application via a set of channels.
///
/// This actor is responsible for forwarding messages from the
/// consensus actor to the application over a channel, and vice-versa.
pub struct Host {}

pub struct HostState {
    pub shard_validator: ShardValidator,
    pub network: NetworkRef<SnapchainValidatorContext>,
    pub consensus_start_delay: u32,
    pub gossip_tx: mpsc::Sender<GossipEvent<SnapchainValidatorContext>>,
    pub statsd: StatsdClientWrapper,
    pub consensus_block_time: u64, // in ms
}

impl Host {
    pub fn new() -> Self {
        Host {}
    }

    pub async fn spawn(
        state: HostState,
    ) -> Result<ActorRef<HostMsg<SnapchainValidatorContext>>, SpawnErr> {
        let (actor_ref, _) = Actor::spawn(None, Self::new(), state).await?;
        Ok(actor_ref)
    }
}

impl Host {
    async fn handle_msg(
        &self,
        _myself: ActorRef<HostMsg<SnapchainValidatorContext>>,
        msg: HostMsg<SnapchainValidatorContext>,
        state: &mut HostState,
    ) -> Result<(), ActorProcessingErr> {
        match msg {
            HostMsg::ConsensusReady(consensus_ref) => {
                // Start height
                state.shard_validator.start(); // Call each time?
                let height = state.shard_validator.get_current_height().increment();
                let validator_set = state.shard_validator.get_validator_set(height.as_u64());
                // Wait a few seconds before starting
                tokio::time::sleep(tokio::time::Duration::from_secs(
                    state.consensus_start_delay as u64,
                ))
                .await;
                info!(
                    height = height.to_string(),
                    validators = validator_set.validators.len(),
                    "Consensus ready. Starting Height"
                );
                consensus_ref.cast(ConsensusMsg::StartHeight(height, validator_set))?;
            }

            HostMsg::StartedRound {
                height,
                round,
                proposer,
            } => {
                state.shard_validator.start_round(height, round, proposer);
                info!(
                    height = height.to_string(),
                    round = round.as_i64(),
                    at = "host_trace",
                    "Started height with round: {}",
                    round.as_i64()
                );
                // Replay undecided values?
            }

            HostMsg::GetValue {
                height,
                round,
                timeout,
                reply_to,
            } => {
                let now = tokio::time::Instant::now();
                let value = state
                    .shard_validator
                    .propose_value(height, round, timeout)
                    .await;
                let shard_hash = value.shard_hash().clone();
                let locally_proposed_value = LocallyProposedValue::new(height, round, shard_hash);
                reply_to.send(locally_proposed_value)?;

                // Next, broadcast the value to the network
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&height.as_u64().to_be_bytes());
                bytes.extend_from_slice(&round.as_i64().to_be_bytes());
                let stream_id = StreamId::new(bytes.into());
                let stream_message = StreamMessage::new(stream_id, 0, StreamContent::Data(value));
                state
                    .network
                    .cast(NetworkMsg::PublishProposalPart(stream_message))?;
                let elapsed = now.elapsed();
                info!(
                    height = height.to_string(),
                    round = round.as_i64(),
                    at = "host_trace",
                    "Proposed value with round: {} ({} ms)",
                    round.as_i64(),
                    elapsed.as_millis()
                );
                state.statsd.time_with_shard(
                    height.shard_index,
                    "host.get_value_time",
                    elapsed.as_millis() as u64,
                );
            }

            HostMsg::RestreamValue {
                height,
                round,
                valid_round,
                address,
                value_id,
            } => {
                // This is only called for pol_rounds which we're not using?
                warn!("RestreamValue at height: {height}, round: {round}, valid_round: {valid_round}, value_id.hash: {:#?}, value_id.shard_index: {:#?}", hex::encode(&value_id.hash), value_id.shard_index);
                let full_proposal = state.shard_validator.get_proposed_value(&value_id);
                match full_proposal {
                    None => {
                        error!(
                            "Could not find previously proposed value for RestreamValue: {}",
                            hex::encode(&value_id.hash)
                        );
                    }
                    Some(full_proposal) => {
                        if full_proposal.height() != height
                            || full_proposal.round() != round
                            || full_proposal.proposer != address.to_vec()
                        {
                            info!(request_height = height.as_u64(), proposal_height = full_proposal.height().as_u64(), request_round = round.as_i64(), proposal_round = full_proposal.round().as_i64(), request_address= hex::encode(address.to_vec()), proposal_address = hex::encode(&full_proposal.proposer), "Proposal published in RestreamValue does not match height/round/proposer in the request")
                        }
                        let mut bytes = Vec::new();
                        bytes.extend_from_slice(&height.as_u64().to_be_bytes());
                        bytes.extend_from_slice(&round.as_i64().to_be_bytes());
                        let stream_id = StreamId::new(bytes.into());
                        let stream_message =
                            StreamMessage::new(stream_id, 0, StreamContent::Data(full_proposal));
                        state
                            .network
                            .cast(NetworkMsg::PublishProposalPart(stream_message))?;
                    }
                }
            }

            HostMsg::GetHistoryMinHeight { reply_to } => {
                reply_to.send(state.shard_validator.get_min_height())?;
            }

            HostMsg::ReceivedProposalPart {
                from,
                part,
                reply_to,
            } => {
                let now = tokio::time::Instant::now();
                let data = part.content.as_data();
                match data {
                    Some(proposal) => {
                        let proposed_value = state
                            .shard_validator
                            .add_proposed_value(proposal, ProposalSource::Consensus);
                        let height = proposed_value.height;
                        let round = proposed_value.round.as_i64();
                        let valid_round = proposed_value.valid_round.as_i64();
                        let is_valid = proposed_value.validity.is_valid();
                        reply_to.send(proposed_value)?;
                        let elapsed = now.elapsed();
                        info!(
                            height = height.to_string(),
                            round = round,
                            at = "host_trace",
                            "Received value at with round: {}, valid_round: {}, valid: {} ({} ms)",
                            round,
                            valid_round,
                            is_valid,
                            elapsed.as_millis()
                        );
                    }
                    None => {
                        error!("Received invalid proposal part from {from}");
                    }
                }
            }

            HostMsg::GetValidatorSet { height, reply_to } => {
                reply_to.send(state.shard_validator.get_validator_set(height.as_u64()))?;
            }

            HostMsg::Decided {
                certificate,
                consensus: consensus_ref,
                extensions: _,
            } => {
                let now = tokio::time::Instant::now();
                let result = state
                    .shard_validator
                    .get_proposed_value(&certificate.value_id);

                if result.is_none() {
                    error!(
                        "Could not find proposed value for decided value: {} at height: {}. Restarting Height.",
                        hex::encode(certificate.value_id.hash),
                        certificate.height
                    );
                    let validator_set = state
                        .shard_validator
                        .get_validator_set(certificate.height.as_u64());
                    consensus_ref
                        .cast(ConsensusMsg::StartHeight(certificate.height, validator_set))?;
                    return Ok(());
                }
                let proposed_value = result.unwrap();

                let commits = Commits::from_commit_certificate(&certificate);
                //commit
                state.shard_validator.decide(commits.clone()).await;

                let decided_value = if let Some(block) = proposed_value.block(commits.clone()) {
                    Some(decided_value::Value::Block(block))
                } else if let Some(shard_chunk) = proposed_value.shard_chunk(commits.clone()) {
                    Some(decided_value::Value::Shard(shard_chunk))
                } else {
                    None
                };

                // Only publish decided values if you're the proposer to reduce network traffic
                if proposed_value.proposer_address() == state.shard_validator.get_address() {
                    state
                        .gossip_tx
                        .send(GossipEvent::BroadcastDecidedValue(proto::DecidedValue {
                            value: decided_value,
                        }))
                        .await
                        .unwrap();
                }

                let elapsed = now.elapsed();
                let height = certificate.height;
                let round = certificate.round;
                info!(
                    height = height.to_string(),
                    round = round.as_i64(),
                    at = "host_trace",
                    "Decided value with round: {} ({} ms)",
                    round.as_i64(),
                    elapsed.as_millis()
                );
                state.statsd.time_with_shard(
                    certificate.height.shard_index,
                    "host.decided_time",
                    elapsed.as_millis() as u64,
                );
                // Start next height, while trying to maintain the block time
                let delay = state
                    .shard_validator
                    .next_height_delay(state.consensus_block_time);
                let next_height = certificate.height.increment();
                let validator_set = state
                    .shard_validator
                    .get_validator_set(next_height.as_u64());
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    if let Err(err) =
                        consensus_ref.cast(ConsensusMsg::StartHeight(next_height, validator_set))
                    {
                        error!(
                            next_height = next_height.as_u64(),
                            "Unable to start next height: {}",
                            err.to_string()
                        );
                    };
                });
            }

            HostMsg::GetDecidedValue { height, reply_to } => {
                info!(height = height.as_u64(), "Get decided value");
                let proposal = state.shard_validator.get_decided_value(height).await;
                let decided_value = match proposal {
                    Some((commits, proposal)) => match proposal {
                        full_proposal::ProposedValue::Block(block) => Some(RawDecidedValue {
                            certificate: commits.to_commit_certificate(),
                            value_bytes: Bytes::from(block.encode_to_vec()),
                        }),
                        full_proposal::ProposedValue::Shard(shard_chunk) => Some(RawDecidedValue {
                            certificate: commits.to_commit_certificate(),
                            value_bytes: Bytes::from(shard_chunk.encode_to_vec()),
                        }),
                    },
                    None => None,
                };
                reply_to.send(decided_value)?;
            }

            HostMsg::ProcessSyncedValue {
                height,
                round,
                validator_address,
                value_bytes,
                reply_to,
            } => {
                let proposal = if height.shard_index == 0 {
                    let decoded_block = Block::decode(value_bytes.as_ref()).unwrap();
                    FullProposal {
                        height: Some(height),
                        round: round.as_i64(),
                        proposer: validator_address.to_vec(),
                        proposed_value: Some(full_proposal::ProposedValue::Block(decoded_block)),
                    }
                } else {
                    let chunk = ShardChunk::decode(value_bytes.as_ref()).unwrap();
                    FullProposal {
                        height: Some(height),
                        round: round.as_i64(),
                        proposer: validator_address.to_vec(),
                        proposed_value: Some(full_proposal::ProposedValue::Shard(chunk)),
                    }
                };
                let proposed_value = state
                    .shard_validator
                    .add_proposed_value(&proposal, ProposalSource::Sync);
                info!(
                    height = height.to_string(),
                    "Processed value via sync: {}", proposed_value.value
                );
                reply_to.send(proposed_value)?;
            }

            // We don't use vote extensions, and don't care about peers joining or leaving here
            HostMsg::ExtendVote {
                height: _,
                round: _,
                value_id: _,
                reply_to,
            } => {
                reply_to.send(None)?;
            }
            HostMsg::VerifyVoteExtension {
                height: _,
                round: _,
                value_id: _,
                extension: _,
                reply_to,
            } => reply_to.send(Ok(()))?,
            HostMsg::PeerJoined { .. } => {}
            HostMsg::PeerLeft { .. } => {}
        };

        Ok(())
    }
}

#[async_trait]
impl Actor for Host {
    type Msg = HostMsg<SnapchainValidatorContext>;
    type State = HostState;
    type Arguments = HostState;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(args)
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        msg: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if let Err(e) = self.handle_msg(myself, msg, state).await {
            error!("Error processing message: {e}");
        }
        Ok(())
    }
}
