use clap::Parser;
use ed25519_dalek::SigningKey;
use snapchain::proto;
use snapchain::proto::admin_service_client::AdminServiceClient;
use snapchain::storage::store::test_helper;
use snapchain::utils::cli;
use snapchain::utils::cli::send_on_chain_event;
use snapchain::utils::factory::events_factory;
use std::error::Error;
use std::{env, panic, process};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// RPC address of the node running the admin service
    #[arg(long, default_value = "http://127.0.0.1:3383")]
    admin_rpc_addr: String,

    /// Authentication credentials for the admin service
    #[arg(long, default_value = "user:test")]
    auth: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env::set_var("RUST_BACKTRACE", "1");
    panic::set_hook(Box::new(|panic_info| {
        eprintln!("Panic occurred: {}", panic_info);
        // let backtrace = std::backtrace::Backtrace::capture();
        // eprintln!("Stack trace:\n{}", backtrace);
        process::exit(1);
    }));

    let args = Args::parse();

    let mut admin_client = AdminServiceClient::connect(args.admin_rpc_addr.clone())
        .await
        .unwrap_or_else(|e| panic!("Error connecting to {}: {}", &args.admin_rpc_addr, e));

    let private_key = test_helper::default_signer();

    // Initialize only two users for testing
    let fids = vec![1_000_001, 1_000_002];

    for fid in fids {
        println!("Initializing user with FID: {}", fid);
        for event in user_events(private_key.clone(), fid) {
            if let Err(e) = send_on_chain_event(&mut admin_client, &event, args.auth.clone()).await
            {
                panic!("Failed to send on-chain event: {:?}", e);
            }
        }
    }

    Ok(())
}

/// Generates a list of on-chain events required to initialize a user with the
/// given FID.
fn user_events(private_key: SigningKey, fid: u64) -> Vec<proto::OnChainEvent> {
    vec![
        cli::compose_rent_event(fid),
        events_factory::create_id_register_event(
            fid,
            proto::IdRegisterEventType::Register,
            vec![],
            None,
        ),
        events_factory::create_signer_event(
            fid,
            private_key,
            proto::SignerEventType::Add,
            None,
            None,
        ),
    ]
}
