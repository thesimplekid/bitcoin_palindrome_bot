use chrono::{NaiveDateTime, TimeZone, Utc};
use nostr_bot::{
    log::debug, tokio, unix_timestamp, wrap, Command, Event, EventNonSigned, FunctorType,
};
use num_format::{Locale, ToFormattedString};
use std::env;
use std::fmt::Write;

mod mempool;

struct Info {
    last_block_hash: String,
    start_timestamp: u64,
}

type State = nostr_bot::State<Info>;

fn format(value: &serde_json::Value) -> String {
    let num = value.to_string().parse::<u64>().unwrap();
    num.to_formatted_string(&Locale::en)
}

async fn get_new_blocks(
    last_block_hash: String,
) -> Result<(String, Vec<serde_json::Value>), String> {
    let current_block_hash = mempool::block_tip_hash()
        .await
        .map_err(|e| format!("Error getting tip hash: {e}"))?;

    debug!(
        "last_block_hash: {}, current_block_hash: {}",
        last_block_hash, current_block_hash
    );
    let mut block_hash = current_block_hash.clone();

    let mut blocks = vec![];
    while block_hash != last_block_hash {
        let block_raw = mempool::get_block(&block_hash)
            .await
            .map_err(|e| format!("Error getting block: {e}"))?;
        debug!("block_raw: >{}<", block_raw);
        let block: serde_json::Value = serde_json::from_str(&block_raw)
            .map_err(|e| format!("Error parsing the block: {e}"))?;
        block_hash = block["previousblockhash"].to_string().replace('\"', "");
        blocks.push(block);
    }

    Ok((current_block_hash, blocks))
}

async fn uptime(event: Event, state: State) -> EventNonSigned {
    let start_timestamp = state.lock().await.start_timestamp;
    let timestamp = unix_timestamp();

    let running_secs = timestamp - start_timestamp;

    nostr_bot::get_reply(
        event,
        format!(
            "Running for {}",
            compound_duration::format_dhms(running_secs)
        ),
    )
}

fn format_blocks(blocks: Vec<serde_json::Value>) -> EventNonSigned {
    let mut content = "".to_string(); // format!("Got {} newly mined block(s):\n", blocks.len());
    let mut tags = vec![vec!["t".to_string(), "bitcoin".to_string()]];
    for (i, block) in blocks.iter().enumerate() {
        let block_height = block["height"].to_string().parse::<u64>().unwrap();
        let next_pal_height = next_pal_height(block_height);
        let last_pal_height = last_pal_height(block_height);
        match is_palindrome(block_height) {
            true => {
                writeln!(
                    content,
                    "Got a newly mined palindrome block :) !: {block_height}"
                )
                .unwrap();
            }
            false => {
                writeln!(
                    content,
                    "Got newly mined block but it wasn't a palindrome :( : {block_height}",
                )
                .unwrap();
            }
        }
        writeln!(content, "Txid: {}", block["id"]).unwrap();
        writeln!(
            content,
            "It has been {} blocks since the last palindrome block {}",
            block_height - last_pal_height,
            last_pal_height
        )
        .unwrap();
        let blocks_to_next = next_pal_height - block_height;
        let min_to_next = blocks_to_next * 10;
        writeln!(
            content,
            "The next palindrome block will be {next_pal_height}, in {blocks_to_next} blocks roughly {min_to_next} minutes"
        )
        .unwrap();
        let block_url = format!(
            "https://mempool.space/block/{}",
            block["id"].to_string().replace('"', "")
        );
        writeln!(content, "- {}", &block_url).unwrap();
        tags.push(vec!["r".to_string(), block_url]);

        if i + 1 < blocks.len() {
            writeln!(content).unwrap();
        }
    }
    EventNonSigned {
        created_at: unix_timestamp(),
        kind: 1,
        content,
        tags,
    }
}

fn is_palindrome(n: u64) -> bool {
    let mut rev = 0;
    let mut x = n;

    while x > 0 {
        rev = rev * 10 + x % 10;
        x /= 10;
    }

    rev == n
}

fn next_pal_height(height: u64) -> u64 {
    let mut x = height;

    while !is_palindrome(x) {
        x += 1;
    }
    x
}

fn last_pal_height(height: u64) -> u64 {
    let mut x = height;

    while !is_palindrome(x) {
        x -= 1;
    }
    x
}

#[tokio::main]
async fn main() {
    nostr_bot::init_logger();

    // let mut secret = std::fs::read_to_string("secret").unwrap();
    // secret.pop(); // Remove newline
    let secret = env::var("SECRET_KEY").unwrap();
    let keypair = nostr_bot::keypair_from_secret(&secret);

    let relays = vec![
        "wss://nostr-pub.wellorder.net",
        "wss://relay.nostr.info",
        "wss://relay.damus.io",
        "wss://nostr.delo.software",
        "wss://nostr.zaprite.io",
        "wss://nostr.zebedee.cloud",
    ];

    let last_block_hash = mempool::block_tip_hash().await.unwrap();

    let state = nostr_bot::wrap_state(Info {
        last_block_hash,
        start_timestamp: unix_timestamp(),
    });

    let sender = nostr_bot::new_sender();

    // TODO: Cleanup
    let update = {
        let sender = sender.clone();
        let state = state.clone();
        async move {
            let errors_discard_period = std::time::Duration::from_secs(3600);
            let mut last_error_time = std::time::SystemTime::now();

            loop {
                let last_block_hash = state.lock().await.last_block_hash.clone();

                match get_new_blocks(last_block_hash).await {
                    Ok((new_block_tip, new_blocks)) => {
                        state.lock().await.last_block_hash = new_block_tip;
                        if !new_blocks.is_empty() {
                            let event = format_blocks(new_blocks);
                            sender.lock().await.send(event.sign(&keypair)).await;
                        }
                    }
                    Err(_e) => {
                        let now = std::time::SystemTime::now();
                        if now.duration_since(last_error_time).unwrap() > errors_discard_period {
                            let event = EventNonSigned {
                                created_at: unix_timestamp(),
                                kind: 1,
                                content: String::from("I'm unable to reach the API."),
                                tags: vec![],
                            };
                            sender.lock().await.send(event.sign(&keypair)).await;
                            last_error_time = now;
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        }
    };

    nostr_bot::Bot::new(keypair, relays, state)
        .name("bitcoin_palindrome_bot")
        .about("Bot publishing info about palindrome blocks. Using https://mempool.space/ API.")
        // .picture("https://upload.wikimedia.org/wikipedia/commons/5/50/Bitcoin.png")
        .command(
            Command::new("!uptime", wrap!(uptime))
                .description("Show for how long is the bot running."),
        )
        .sender(sender)
        .spawn(Box::pin(update))
        .help()
        .run()
        .await;
}
