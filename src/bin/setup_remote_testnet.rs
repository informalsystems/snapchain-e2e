use clap::Parser;
use libp2p::identity::ed25519::{Keypair, SecretKey};
use serde::Deserialize;
use std::fs;
use std::time::Duration;
use toml::Value;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "./nodes/infra-data.json")]
    infra_path: String,

    /// Delay between blocks (e.g. "250ms")
    #[arg(long, value_parser = parse_duration, default_value = "1000ms")]
    block_time: Duration,

    #[arg(long, default_value = "")]
    l1_rpc_url: String,

    #[arg(long, default_value = "")]
    l2_rpc_url: String,

    #[arg(long, default_value = "108864739")]
    start_block_number: u64,

    #[arg(long)]
    stop_block_number: Option<u64>,

    /// Statsd prefix. note: node ID will be appended before config file written
    #[arg(long, default_value = "snapchain")]
    statsd_prefix: String,

    #[arg(long, default_value = "false")]
    statsd_use_tags: bool,

    #[arg(long, default_value = "")]
    snapshot_endpoint_url: String,

    #[arg(long, default_value = "")]
    aws_access_key_id: String,

    #[arg(long, default_value = "")]
    aws_secret_access_key: String,

    #[arg(long, default_value = "2")]
    num_shards: u32,

    #[arg(
        long,
        default_value = "20",
        help = "Number of full nodes that will be initially connected to validators"
    )]
    first_full_nodes: u64,

    // Values: "default", "sparse", "groups", "small"
    #[arg(long, default_value = "default")]
    topology: String,
    // #[arg(long, default_value = "5")]
    // num_validators: u32,

    // #[arg(long, default_value = "20")]
    // num_full_nodes: u32,
}

fn parse_duration(arg: &str) -> Result<Duration, String> {
    humantime::parse_duration(arg).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct InfraData {
    cc: NodeInstance,
    instances: std::collections::HashMap<String, NodeInstance>,
    num_validators: u64,
    num_full_nodes: u64,
}

#[derive(Deserialize)]
struct NodeInstance {
    // _public_ip: String,
    private_ip: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let json_data = fs::read_to_string(args.infra_path).expect("Failed to read json file");
    let infra: InfraData = serde_json::from_str(&json_data).expect("json parsing error");

    let num_total_nodes = infra.num_validators + infra.num_full_nodes;

    // create directory at the root of the project if it doesn't exist
    if !std::path::Path::new("nodes").exists() {
        std::fs::create_dir("nodes").expect("Failed to create nodes directory");
    }

    let keypairs = (1..=num_total_nodes)
        .map(|_| SecretKey::generate())
        .collect::<Vec<SecretKey>>();
    let all_public_keys = keypairs
        .iter()
        .map(|x| hex::encode(Keypair::from(x.clone()).public().to_bytes()))
        .collect::<Vec<String>>();
    let validator_addresses = Value::Array(
        all_public_keys
            .iter()
            .take(infra.num_validators as usize)
            .map(|x| Value::String(x.clone()))
            .collect(),
    )
    .to_string();

    let default_gossip_port = 3382;
    let statsd_ip = infra.cc.private_ip.clone();

    // Create a config file for the validators
    for i in 1..=infra.num_validators {
        if !std::path::Path::new(format!("nodes/val{i}").as_str()).exists() {
            std::fs::create_dir(format!("nodes/val{i}")).expect("Failed to create node directory");
        }
        let secret_key = hex::encode(&keypairs[i as usize - 1]);
        let host = format!("0.0.0.0");
        let gossip_multi_addr = format!("/ip4/{host}/udp/{default_gossip_port}/quic-v1");

        // Validators are connected in a full mesh
        let other_nodes_addresses = (1..=infra.num_validators)
            .filter(|&j| j != i)
            .map(|j| {
                format!(
                    "/ip4/{}/udp/{default_gossip_port}/quic-v1",
                    infra
                        .instances
                        .get(format!("val{j}").as_str())
                        .expect("validator index out of bounds")
                        .private_ip
                )
            })
            .collect::<Vec<String>>()
            .join(",");

        let block_time = humantime::format_duration(args.block_time);
        let num_shards = args.num_shards;
        let shard_ids = format!(
            "[{}]",
            (1..=num_shards)
                .map(|x| x.to_string())
                .collect::<Vec<String>>()
                .as_slice()
                .join(",")
        );

        let validator_sets = format!(
            "{{ effective_at = 0, validator_public_keys = {}, shard_ids = {} }}",
            validator_addresses, shard_ids,
        );

        let statsd_prefix = format!("{}{}", args.statsd_prefix, i);
        let statsd_use_tags = args.statsd_use_tags;
        let l1_rpc_url = args.l1_rpc_url.clone();
        let l2_rpc_url = args.l2_rpc_url.clone();
        let start_block_number = args.start_block_number;
        let snapshot_endpoint_url = args.snapshot_endpoint_url.clone();
        let aws_access_key_id = args.aws_access_key_id.clone();
        let aws_secret_access_key = args.aws_secret_access_key.clone();
        let stop_block_number = match args.stop_block_number {
            None => "".to_string(),
            Some(number) => format!("stop_block_number = {number}").to_string(),
        };

        let config_file_content = format!(
            r#"
rpc_address="0.0.0.0:3381"
http_address="0.0.0.0:3383"
rocksdb_dir="/app/data/.rocks"
l1_rpc_url="{l1_rpc_url}"

[statsd]
prefix="{statsd_prefix}"
addr="{statsd_ip}:8125"
use_tags={statsd_use_tags}

[gossip]
address="{gossip_multi_addr}"
bootstrap_peers = "{other_nodes_addresses}"

[consensus]
private_key = "{secret_key}"
block_time = "{block_time}"
shard_ids = {shard_ids}
num_shards = {num_shards}
validator_sets = [{validator_sets}]

[onchain_events]
rpc_url= "{l2_rpc_url}"
start_block_number = {start_block_number}
{stop_block_number}

[snapshot]
endpoint_url = "{snapshot_endpoint_url}"
backup_dir = "/app/data/.rocks.backup"
snapshot_download_dir = "/app/data/.rocks.snapshot"
load_db_from_snapshot=false
aws_access_key_id = "{aws_access_key_id}"
aws_secret_access_key = "{aws_secret_access_key}"
            "#
        );

        // clean up whitespace
        let config_file_content = config_file_content.trim().to_string() + "\n";

        std::fs::write(format!("nodes/val{i}/config.toml"), config_file_content)
            .expect("Failed to write config file");
        // Print a message
    }

    let mut validator_idx = 1;

    let mut group_1_idx = 1;
    let mut group_2_idx = 2;

    let mut full_node_idx = 1;

    // Create a config file for the full nodes
    for i in 1..=infra.num_full_nodes {
        if !std::path::Path::new(format!("nodes/full{i}").as_str()).exists() {
            std::fs::create_dir(format!("nodes/full{i}")).expect("Failed to create node directory");
        }
        let host = format!("0.0.0.0");
        let gossip_multi_addr = format!("/ip4/{host}/udp/{default_gossip_port}/quic-v1");

        let mut other_nodes_addresses = Vec::new();

        match args.topology.as_str() {
            "default" => {
                // Connect to 2 validators in round robin based on full node id, only for first_full_nodes
                if i < args.first_full_nodes {
                    for _ in 0..2 {
                        let val = infra
                            .instances
                            .get(format!("val{validator_idx}").as_str())
                            .expect("validator index out of bounds");
                        if !other_nodes_addresses.contains(&val.private_ip) {
                            other_nodes_addresses.push(val.private_ip.clone());
                        }
                        validator_idx = (validator_idx % infra.num_validators) + 1;
                    }
                }
                // Connect to 2 other full nodes: the next ones in id order (wrapping around)
                for _ in 0..2 {
                    if full_node_idx != i {
                        let node = infra
                            .instances
                            .get(format!("full{full_node_idx}").as_str())
                            .expect("full node index out of bounds");
                        if !other_nodes_addresses.contains(&node.private_ip) {
                            other_nodes_addresses.push(node.private_ip.clone());
                        }
                    }
                    full_node_idx = (full_node_idx % infra.num_full_nodes) + 1;
                }
            }
            "sparse" => {
                // 25% of the full nodes are connected to 2 validators
                // The rest are connected to 4 full nodes from the previous group
                if i <= (infra.num_full_nodes / 4) {
                    // Connect to 2 validators
                    for _ in 0..2 {
                        let val = infra
                            .instances
                            .get(format!("val{validator_idx}").as_str())
                            .expect("validator index out of bounds");
                        if !other_nodes_addresses.contains(&val.private_ip) {
                            other_nodes_addresses.push(val.private_ip.clone());
                        }
                        validator_idx = (validator_idx % infra.num_validators) + 1;
                    }
                } else {
                    // Connect to 4 full nodes from the previous group
                    for _ in 0..4 {
                        let node = infra
                            .instances
                            .get(format!("full{full_node_idx}").as_str())
                            .expect("full node index out of bounds");
                        if !other_nodes_addresses.contains(&node.private_ip) {
                            other_nodes_addresses.push(node.private_ip.clone());
                        }
                        full_node_idx = (full_node_idx % (infra.num_full_nodes / 4)) + 1;
                    }
                }
            }
            "groups" => {
                // 3 groups
                // Connect group 1 to 2 validators
                // Connect group 2 to 2 full nodes from group 1
                // Connect group 3 to 2 full nodes from group 2
                match (i - 1) % 3 {
                    0 => {
                        // Connect to 2 validators
                        for _ in 0..1 {
                            let val = infra
                                .instances
                                .get(format!("val{validator_idx}").as_str())
                                .expect("validator index out of bounds");
                            if !other_nodes_addresses.contains(&val.private_ip) {
                                other_nodes_addresses.push(val.private_ip.clone());
                            }
                            validator_idx = (validator_idx % infra.num_validators) + 1;
                        }
                    }
                    1 => {
                        // Connect to 2 full nodes from group 1
                        for _ in 0..2 {
                            let node = infra
                                .instances
                                .get(format!("full{group_1_idx}").as_str())
                                .expect("full node index out of bounds");
                            if !other_nodes_addresses.contains(&node.private_ip) {
                                other_nodes_addresses.push(node.private_ip.clone());
                            }
                            if group_1_idx + 2 >= infra.num_full_nodes {
                                group_1_idx = 1;
                            } else {
                                group_1_idx = ((group_1_idx + 2) % infra.num_full_nodes) + 1;
                            }
                        }
                    }
                    2 => {
                        // Connect to 2 full nodes from group 2
                        for _ in 0..2 {
                            let node = infra
                                .instances
                                .get(format!("full{group_2_idx}").as_str())
                                .expect("full node index out of bounds");
                            if !other_nodes_addresses.contains(&node.private_ip) {
                                other_nodes_addresses.push(node.private_ip.clone());
                            }
                            if group_2_idx + 2 >= infra.num_full_nodes {
                                group_2_idx = 2;
                            } else {
                                group_2_idx = ((group_2_idx + 2) % infra.num_full_nodes) + 1;
                            }
                        }
                    }
                    _ => panic!("Unexpected group assignment"),
                }
            }
            "small" => {
                if infra.num_full_nodes != 3 {
                    panic!("The 'small' topology is only supported for 3 full nodes");
                }
                // Connect to all validators
                for j in 1..=infra.num_validators {
                    let val = infra
                        .instances
                        .get(format!("val{j}").as_str())
                        .expect("validator index out of bounds");
                    if !other_nodes_addresses.contains(&val.private_ip) {
                        other_nodes_addresses.push(val.private_ip.clone());
                    }
                }

                match i {
                    1 => {}
                    2 => {
                        // Connect to full node 3
                        let node = infra
                            .instances
                            .get("full3")
                            .expect("full node index out of bounds");
                        if !other_nodes_addresses.contains(&node.private_ip) {
                            other_nodes_addresses.push(node.private_ip.clone());
                        }
                    }
                    3 => {
                        // Connect to full node 2
                        let node = infra
                            .instances
                            .get("full2")
                            .expect("full node index out of bounds");
                        if !other_nodes_addresses.contains(&node.private_ip) {
                            other_nodes_addresses.push(node.private_ip.clone());
                        }
                    }
                    _ => {
                        panic!("Unexpected full node index for 'small' topology: {}", i);
                    }
                }
            }
            _ => {
                panic!("Unknown topology: {}", args.topology);
            }
        }

        other_nodes_addresses = other_nodes_addresses
            .iter()
            .map(|addr| format!("/ip4/{}/udp/{}/quic-v1", addr, default_gossip_port))
            .collect();

        let other_nodes_addresses = other_nodes_addresses.join(",");

        let num_shards = args.num_shards;
        let shard_ids = format!(
            "[{}]",
            (1..=num_shards)
                .map(|x| x.to_string())
                .collect::<Vec<String>>()
                .as_slice()
                .join(",")
        );

        let validator_sets = format!(
            "{{ effective_at = 0, validator_public_keys = {}, shard_ids = {} }}",
            validator_addresses, shard_ids,
        );

        let statsd_prefix = format!("{}{}", args.statsd_prefix, i);
        let statsd_use_tags = args.statsd_use_tags;
        let l1_rpc_url = args.l1_rpc_url.clone();
        let l2_rpc_url = args.l2_rpc_url.clone();
        let start_block_number = args.start_block_number;
        let snapshot_endpoint_url = args.snapshot_endpoint_url.clone();
        let aws_access_key_id = args.aws_access_key_id.clone();
        let aws_secret_access_key = args.aws_secret_access_key.clone();
        let stop_block_number = match args.stop_block_number {
            None => "".to_string(),
            Some(number) => format!("stop_block_number = {number}").to_string(),
        };

        let config_file_content = format!(
            r#"
rpc_address="0.0.0.0:3381"
http_address="0.0.0.0:3383"
rocksdb_dir="/app/data/.rocks"
l1_rpc_url="{l1_rpc_url}"
read_node=true

[statsd]
prefix="{statsd_prefix}"
addr="{statsd_ip}:8125"
use_tags={statsd_use_tags}

[gossip]
address="{gossip_multi_addr}"
bootstrap_peers = "{other_nodes_addresses}"

[consensus]
shard_ids = {shard_ids}
num_shards = {num_shards}
validator_sets = [{validator_sets}]

[onchain_events]
rpc_url= "{l2_rpc_url}"
start_block_number = {start_block_number}
{stop_block_number}

[snapshot]
endpoint_url = "{snapshot_endpoint_url}"
backup_dir = "/app/data/.rocks.backup"
snapshot_download_dir = "/app/data/.rocks.snapshot"
load_db_from_snapshot=false
aws_access_key_id = "{aws_access_key_id}"
aws_secret_access_key = "{aws_secret_access_key}"
            "#
        );

        // clean up whitespace
        let config_file_content = config_file_content.trim().to_string() + "\n";

        std::fs::write(format!("nodes/full{i}/config.toml"), config_file_content)
            .expect("Failed to write config file");
        // Print a message
    }

    println!("Created configs for {num_total_nodes} nodes");
}
