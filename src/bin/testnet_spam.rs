use clap::Parser;
use core::fmt;
use ed25519_dalek::SigningKey;
use rand::{distributions::Alphanumeric, Rng};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::{self, sleep, Duration, Instant};

use snapchain::proto::hub_service_client::HubServiceClient;
use snapchain::proto::{self};
use snapchain::storage::store::test_helper;
use snapchain::utils::cli;

type Result<T, E = Box<dyn std::error::Error + Send + Sync>> = core::result::Result<T, E>;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "http://127.0.0.1:3383")]
    addr: String,

    #[arg(long, default_value = "0")]
    max_msgs: u64,

    #[arg(long, default_value = "0")]
    max_time: u64,

    #[arg(long, default_value = "100")]
    rate: u64,

    #[arg(long, default_value = "140")]
    msg_size: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    let spammer = Spammer::new(
        "spammer".to_string(),
        args.addr,
        args.max_msgs,
        args.max_time,
        args.rate,
        args.msg_size,
    )
    .await?;

    spammer.run().await
}

/// A spammer that sends messages at a controlled rate.
/// Tracks and reports statistics on sent messages.
pub struct Spammer {
    /// Spammer identifier (in case of multiple spammer instances).
    id: String,
    /// RPC address of the target node.
    rpc_addr: String,
    /// Maximum number of messages to send (0 for no limit).
    max_msgs: u64,
    /// Maximum number of seconds to run the spammer (0 for no limit).
    /// If both max_msgs and max_time are 0, the spammer will run indefinitely.
    max_time: u64,
    /// Number of messages to send per second.
    rate: u64,
    /// Number of characters in each message.
    msg_size: u64,
}

impl Spammer {
    pub async fn new(
        id: String,
        rpc_addr: String,
        max_msgs: u64,
        max_time: u64,
        rate: u64,
        msg_size: u64,
    ) -> Result<Self> {
        Ok(Self {
            id,
            rpc_addr,
            max_msgs,
            max_time,
            rate,
            msg_size,
        })
    }

    pub async fn run(self) -> Result<()> {
        println!(
            "[{}] rpc_addr={}, max_msgs={}, max_time={}, rate={}, msg_size={}",
            self.id, self.rpc_addr, self.max_msgs, self.max_time, self.rate, self.msg_size
        );

        // Create communication channels between spammer and result tracker.
        let (result_sender, result_receiver) = mpsc::channel::<Result<usize>>(10000);
        let (report_sender, report_receiver) = mpsc::channel::<Instant>(1);
        let (finish_sender, finish_receiver) = mpsc::channel::<()>(1);

        let self_arc = Arc::new(self);

        // Spawn spammer.
        let spammer_handle = tokio::spawn({
            let self_arc = Arc::clone(&self_arc);
            async move {
                self_arc
                    .spammer(result_sender, report_sender, finish_sender)
                    .await
            }
        });

        // Spawn result tracker.
        let tracker_handle = tokio::spawn({
            let self_arc = Arc::clone(&self_arc);
            async move {
                self_arc
                    .tracker(result_receiver, report_receiver, finish_receiver)
                    .await
            }
        });

        let _ = tokio::join!(spammer_handle, tracker_handle);
        Ok(())
    }

    /// Spammer thread that generates and sends messages to the node at a controlled rate.
    async fn spammer(
        &self,
        result_sender: Sender<Result<usize>>,
        report_sender: Sender<Instant>,
        finish_sender: Sender<()>,
    ) -> Result<()> {
        // Connect to the node.
        let mut msg_factory =
            MsgFactory::new(1_000_001, test_helper::default_signer(), self.msg_size);
        let mut client = HubServiceClient::connect(self.rpc_addr.clone())
            .await
            .unwrap_or_else(|e| panic!("Error connecting to {}: {}", &self.rpc_addr, e));

        // Initialize counters.
        let start_time = Instant::now();
        let mut txs_sent_total = 0u64;
        let mut interval = time::interval(Duration::from_secs(1));
        loop {
            // Wait for next one-second tick.
            let _ = interval.tick().await;
            let mut txs_sent_in_interval = 0u64;
            let interval_start = Instant::now();

            // Send up to `rate` transactions per one-second interval.
            while txs_sent_in_interval < self.rate {
                // Check exit conditions before sending each transaction.
                if self.should_stop(start_time, txs_sent_total) {
                    break;
                }

                // Create and send a message.
                let msg = msg_factory.make_msg().await;
                let result = cli::send_message(&mut client, &msg, None)
                    .await
                    .map(|msg| format!("{:?}", msg).len()) // TODO: compute message size properly
                    .map_err(|s| format!("Server Error {}: {}", s.code(), s.message()).into());

                // Report result and update counters.
                result_sender.send(result).await?;
                txs_sent_in_interval += 1;
                txs_sent_total += 1;
            }

            // Give time to the in-flight results to be received.
            sleep(Duration::from_millis(20)).await;

            // Signal tracker to report stats after this batch.
            report_sender.try_send(interval_start)?;

            // Check exit conditions after each tick.
            if self.should_stop(start_time, txs_sent_total) {
                break;
            }
        }

        // Signal tracker to finish.
        finish_sender.send(()).await?;

        Ok(())
    }

    /// Check if spammer exceeded the maximum number of messages or time limit.
    fn should_stop(&self, start_time: Instant, txs_sent_total: u64) -> bool {
        (self.max_msgs > 0 && txs_sent_total >= self.max_msgs)
            || (self.max_time > 0 && start_time.elapsed().as_secs() >= self.max_time)
    }

    /// Result tracker thread that receives and aggregates statistics on sent messages every second.
    async fn tracker(
        &self,
        mut result_receiver: Receiver<Result<usize>>,
        mut report_receiver: Receiver<Instant>,
        mut finish_receiver: Receiver<()>,
    ) -> Result<()> {
        // Initialize counters
        let start_time = Instant::now();
        let mut stats_total = Stats::new(self.id.clone(), start_time);
        let mut stats_last_second = Stats::new(self.id.clone(), start_time);
        loop {
            tokio::select! {
                // Update counters
                Some(res) = result_receiver.recv() => {
                    match res {
                        Ok(tx_length) => stats_last_second.incr_ok(tx_length),
                        Err(error) => stats_last_second.incr_err(&error.to_string()),
                    }
                }
                // Report stats
                Some(interval_start) = report_receiver.recv() => {
                    // Wait what's missing to complete one second.
                    let elapsed = interval_start.elapsed();
                    if elapsed < Duration::from_secs(1) {
                        sleep(Duration::from_secs(1) - elapsed).await;
                    }

                    println!("{stats_last_second}");

                    // Update total, then reset last second stats
                    stats_total.add(&stats_last_second);
                    stats_last_second.reset();
                }
                // Stop tracking on signal from spammer.
                _ = finish_receiver.recv() => {
                    break;
                }
            }
        }
        println!(
            "Total: {stats_total}, rate: {:.1} msg/s, {:.1} byte/s",
            stats_total.rate_msgs(),
            stats_total.rate_bytes()
        );
        Ok(())
    }
}

/// Statistics on sent messages.
struct Stats {
    id: String,
    start_time: Instant,
    succeed: u64,
    bytes: usize,
    errors_counter: HashMap<String, u64>,
}

impl Stats {
    fn new(id: String, start_time: Instant) -> Self {
        Self {
            id,
            start_time,
            succeed: 0,
            bytes: 0,
            errors_counter: HashMap::new(),
        }
    }

    fn incr_ok(&mut self, tx_length: usize) {
        self.succeed += 1;
        self.bytes += tx_length;
    }

    fn incr_err(&mut self, error: &str) {
        self.errors_counter
            .entry(error.to_string())
            .and_modify(|count| *count += 1)
            .or_insert(1);
    }

    fn add(&mut self, other: &Self) {
        self.succeed += other.succeed;
        self.bytes += other.bytes;
        for (error, count) in &other.errors_counter {
            self.errors_counter
                .entry(error.to_string())
                .and_modify(|c| *c += count)
                .or_insert(*count);
        }
    }

    fn reset(&mut self) {
        self.succeed = 0;
        self.bytes = 0;
        self.errors_counter.clear();
    }

    fn rate_msgs(&self) -> f64 {
        self.succeed as f64 / self.start_time.elapsed().as_millis() as f64 * 1000f64
    }

    fn rate_bytes(&self) -> f64 {
        self.bytes as f64 / self.start_time.elapsed().as_millis() as f64 * 1000f64
    }
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let elapsed = self.start_time.elapsed().as_millis();
        let stats = format!(
            "[{}] elapsed {:.3}s: Sent {} messages ({} bytes)",
            self.id,
            elapsed as f64 / 1000f64,
            self.succeed,
            self.bytes,
        );
        let stats_failed = if self.errors_counter.is_empty() {
            String::new()
        } else {
            let failed = self.errors_counter.values().map(|c| *c).sum::<u64>();
            format!("; {} failed with {:?}", failed, self.errors_counter)
        };
        write!(f, "{stats}{stats_failed}")
    }
}

struct MsgFactory {
    // User ID.
    fid: u64,
    // Private key of the user.
    private_key: SigningKey,
    // Message counter.
    i: u64,
    // Size of each message in bytes.
    msg_bytes: u64,
}

impl MsgFactory {
    pub fn new(fid: u64, private_key: SigningKey, msg_bytes: u64) -> Self {
        MsgFactory {
            fid,
            private_key,
            i: 0,
            msg_bytes,
        }
    }

    pub async fn make_msg(&mut self) -> proto::Message {
        let mut content = format!("test-message-{}--", self.i);

        // Fill the content with random bytes to reach the desired size.
        let bytes_to_add = self.msg_bytes as usize - content.len();
        if bytes_to_add > 0 {
            let filler: String = rand::thread_rng()
                .sample_iter(&Alphanumeric)
                .take(bytes_to_add)
                .map(char::from)
                .collect();
            content.push_str(&filler);
        }

        self.i += 1;
        cli::compose_message(self.fid, &content, None, Some(&self.private_key))
    }
}
