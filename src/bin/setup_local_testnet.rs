use clap::Parser;
use libp2p::identity::ed25519::{Keypair, SecretKey};
use std::time::Duration;
use toml::Value;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Delay between blocks (e.g. "250ms")
    #[arg(long, value_parser = parse_duration, default_value = "250ms")]
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

    #[arg(long, default_value = "127.0.0.1:8125")]
    statsd_addr: String,

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

    #[arg(long, default_value = "4")]
    num_nodes: u32,
}

fn parse_duration(arg: &str) -> Result<Duration, String> {
    humantime::parse_duration(arg).map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let num_nodes = args.num_nodes;

    // create directory at the root of the project if it doesn't exist
    if !std::path::Path::new("nodes").exists() {
        std::fs::create_dir("nodes").expect("Failed to create nodes directory");
    }

    let keypairs = (1..=num_nodes)
        .map(|_| SecretKey::generate())
        .collect::<Vec<SecretKey>>();
    let all_public_keys = keypairs
        .iter()
        .map(|x| hex::encode(Keypair::from(x.clone()).public().to_bytes()))
        .collect::<Vec<String>>();
    let validator_addresses = Value::Array(
        all_public_keys
            .iter()
            .map(|x| Value::String(x.clone()))
            .collect(),
    )
    .to_string();

    let base_rpc_port = 3382;
    let base_http_port = 3482;
    let base_gossip_port = 50050;
    for i in 1..=num_nodes {
        let id = i;
        let db_dir = format!("nodes/{id}/.rocks");
        let backup_dir = format!("nodes/{id}/.rocks.backup");
        let snapshot_download_dir = format!("nodes/{id}/.rocks.snapshot");

        if !std::path::Path::new(format!("nodes/{id}").as_str()).exists() {
            std::fs::create_dir(format!("nodes/{id}")).expect("Failed to create node directory");
        } else {
            if std::path::Path::new(db_dir.clone().as_str()).exists() {
                std::fs::remove_dir_all(db_dir.clone()).expect("Failed to remove .rocks directory");
            }
        }
        let secret_key = hex::encode(&keypairs[i as usize - 1]);
        let rpc_port = base_rpc_port + i;
        let http_port = base_http_port + i;
        let gossip_port = base_gossip_port + i;
        let host = format!("0.0.0.0");
        let rpc_address = format!("{host}:{rpc_port}");
        let http_address = format!("{host}:{http_port}");
        let gossip_multi_addr = format!("/ip4/{host}/udp/{gossip_port}/quic-v1");
        let other_nodes_addresses = (1..=num_nodes)
            .filter(|&x| x != id)
            .map(|x| {
                format!(
                    "/ip4/172.100.0.1{}/udp/{:?}/quic-v1",
                    x,
                    base_gossip_port + x
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

        let statsd_prefix = format!("{}{}", args.statsd_prefix, id);
        let statsd_addr = args.statsd_addr.clone();
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
rpc_address="{rpc_address}"
http_address="{http_address}"
rocksdb_dir="{db_dir}"
l1_rpc_url="{l1_rpc_url}"

[statsd]
prefix="{statsd_prefix}"
addr="{statsd_addr}"
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
backup_dir = "{backup_dir}"
snapshot_download_dir = "{snapshot_download_dir}"
load_db_from_snapshot=false
aws_access_key_id = "{aws_access_key_id}"
aws_secret_access_key = "{aws_secret_access_key}"
            "#
        );

        // clean up whitespace
        let config_file_content = config_file_content.trim().to_string() + "\n";

        std::fs::write(
            format!("nodes/{id}/snapchain.toml", id = id),
            config_file_content,
        )
        .expect("Failed to write config file");
        // Print a message
    }
    println!("Created configs for {num_nodes} nodes");
}
