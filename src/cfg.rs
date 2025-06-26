use crate::{
    connectors::{self},
    consensus, mempool,
    network::{self, http_server},
    proto::FarcasterNetwork,
    storage,
};
use clap::Parser;
use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Deserialize, Serialize)]
pub struct StatsdConfig {
    pub prefix: String,
    pub addr: String,
    pub use_tags: bool,
}

impl Default for StatsdConfig {
    fn default() -> Self {
        Self {
            prefix: "".to_string(), //TODO: "snapchain" eventually
            addr: "127.0.0.1:8125".to_string(),
            use_tags: true,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PruningConfig {
    #[serde(
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none",
        default // TODO: for now defaults to None, but should be 1mo.
    )]
    pub block_retention: Option<Duration>,
    #[serde(with = "humantime_serde")]
    pub event_retention: Duration,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            block_retention: None,
            event_retention: Duration::from_secs(60 * 60 * 24 * 3), // 3 days
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub log_format: String,
    pub fnames: connectors::fname::Config,
    pub onchain_events: connectors::onchain_events::Config,
    pub base_onchain_events: connectors::onchain_events::Config,
    pub consensus: consensus::consensus::Config,
    pub gossip: network::gossip::Config,
    pub mempool: mempool::mempool::Config,
    pub snapshot: storage::db::snapshot::Config,
    pub rpc_auth: String,
    pub admin_rpc_auth: String,
    pub rpc_address: String,
    pub http_address: String,
    pub rocksdb_dir: String,
    pub clear_db: bool,
    pub statsd: StatsdConfig,
    pub trie_branching_factor: u32,
    pub l1_rpc_url: String,
    pub fc_network: FarcasterNetwork,
    pub read_node: bool,
    pub pruning: PruningConfig,
    pub http_server: http_server::Config,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            log_format: "text".to_string(),
            fnames: connectors::fname::Config::default(),
            onchain_events: connectors::onchain_events::Config::default(),
            base_onchain_events: connectors::onchain_events::Config::default(),
            consensus: consensus::consensus::Config::default(),
            gossip: network::gossip::Config::default(),
            mempool: mempool::mempool::Config::default(),
            rpc_auth: "".to_string(),
            admin_rpc_auth: "".to_string(),
            rpc_address: "0.0.0.0:3383".to_string(),
            http_address: "0.0.0.0:3381".to_string(),
            rocksdb_dir: ".rocks".to_string(),
            clear_db: false,
            statsd: StatsdConfig::default(),
            trie_branching_factor: 16,
            l1_rpc_url: "".to_string(),
            fc_network: FarcasterNetwork::Devnet,
            snapshot: storage::db::snapshot::Config::default(),
            read_node: false,
            pruning: PruningConfig::default(),
            http_server: http_server::Config::default(),
        }
    }
}

#[derive(Parser)]
pub struct CliArgs {
    #[arg(long, help = "Log format (text or json)")]
    log_format: Option<String>,

    #[arg(long, help = "Path to the config file")]
    config_path: String,

    #[arg(long, action, help = "Start the node with a clean database")]
    clear_db: bool,
    // All new arguments that are to override values from config files or environment variables
    // should be probably be optional (`Option<T>`) and without a default. Setting a default
    // in this case will have the effect of automatically overriding all previous configuration
    // layers. Remember to add the override code below and a test case.
}

pub fn load_and_merge_config(args: Vec<String>) -> Result<Config, Box<dyn Error>> {
    let cli_args = CliArgs::try_parse_from(args)?;

    let mut figment = Figment::from(Serialized::defaults(Config::default()));

    if Path::new(&cli_args.config_path).exists() {
        figment = figment.merge(Toml::file(&cli_args.config_path));
    } else {
        return Err(format!("config file not found: {}", &cli_args.config_path).into());
    }

    figment = figment.merge(Env::prefixed("SNAPCHAIN_").split("__"));

    let mut config: Config = figment.extract()?;

    if let Some(log_format) = cli_args.log_format {
        config.log_format = log_format;
    }
    config.clear_db = cli_args.clear_db;

    Ok(config)
}
