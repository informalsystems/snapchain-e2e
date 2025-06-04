use clap::Parser;
use snapchain::proto::hub_service_client::HubServiceClient;
use snapchain::storage::store::test_helper;
use snapchain::utils::cli;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "http://127.0.0.1:3383")]
    addr: String,

    #[arg(long, default_value = "100")]
    num: usize,
}

#[tokio::main]
async fn main() {
    let args = Cli::parse();
    let rpc_addr = args.addr;
    let num = args.num;

    let private_key = test_helper::default_signer();

    let mut client = HubServiceClient::connect(rpc_addr.clone())
        .await
        .unwrap_or_else(|e| panic!("Error connecting to {}: {}", &rpc_addr, e));

    // Fixed user FID for testing
    let fid = 1_000_001;

    let mut success = 0;
    for i in 1..num + 1 {
        let text = format!("Test message: {}", i);
        let msg = cli::compose_message(fid, &text, None, Some(&private_key));
        let resp = cli::send_message(&mut client, &msg, None).await;

        if resp.is_ok() {
            success += 1;
        } else {
            eprintln!("Failed to send message {}: {:?}", i, resp.err());
        }
    }

    println!("Submitted {} messages, {} succeeded", num, success);
}
