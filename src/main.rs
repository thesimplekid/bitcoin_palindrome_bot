use dotenvy::dotenv;
use nostr_bot::{
    log::debug, tokio, unix_timestamp, wrap, Command, Event, EventNonSigned, FunctorType,
};
use num_format::{Locale, ToFormattedString};
use std::env;
use std::fmt::Write;

use anyhow::Result;

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

fn convert_minutes(minutes: u64) -> (u64, u64, u64) {
    let days = minutes / 1440;
    let hours = (minutes % 1440) / 60;
    let remaining_minutes = minutes % 60;
    (days, hours, remaining_minutes)
}

fn format_blocks(blocks: Vec<serde_json::Value>) -> Result<EventNonSigned> {
    let mut content = "".to_string(); // format!("Got {} newly mined block(s):\n", blocks.len());
    let mut tags = vec![vec!["t".to_string(), "bitcoin".to_string()]];
    for (i, block) in blocks.iter().enumerate() {
        let block_height = block["height"].to_string().parse::<u64>()?;
        let next_pal_height = next_pal_height(block_height);
        let last_pal_height = last_pal_height(block_height);
        let block_height_str = block_height.to_formatted_string(&Locale::en);
        match is_palindrome(block_height) {
            true => {
                writeln!(content, "!!! Palindrome Block Mined !!!")?;
                writeln!(
                    content,
                    "Block {block_height_str} was just mined and is a palindrome :) !"
                )?;
            }
            false => {
                writeln!(
                    content,
                    "Block {block_height_str} was just mined block but it wasn't a palindrome :(",
                )?;
            }
        }
        writeln!(content)?;

        let blocks_since = block_height - last_pal_height;
        writeln!(
            content,
            "It has been {} blocks since the last palindrome block {}",
            blocks_since.to_formatted_string(&Locale::en),
            last_pal_height.to_formatted_string(&Locale::en)
        )?;

        let blocks_to_next = next_pal_height - block_height;
        let min_to_next = blocks_to_next * 10;
        let (days, hours, minutes) = convert_minutes(min_to_next);
        writeln!(
            content,
            "The next palindrome block will be {}, in {} blocks roughly {days} day(s) {hours} hour(s) and {minutes} minutes",
            next_pal_height.to_formatted_string(&Locale::en),
            blocks_to_next.to_formatted_string(&Locale::en)
            )?;

        let block_url = format!(
            "https://mempool.space/block/{}",
            block["id"].to_string().replace('"', "")
        );
        writeln!(content, "- {}", &block_url)?;
        tags.push(vec!["r".to_string(), block_url]);

        if i + 1 < blocks.len() {
            writeln!(content)?;
        }
    }
    Ok(EventNonSigned {
        created_at: unix_timestamp(),
        kind: 1,
        content,
        tags,
    })
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

    dotenv().expect(".env file not found");

    let secret = env::var("SECRET_KEY").unwrap();
    let keypair = nostr_bot::keypair_from_secret(&secret);

    let relays = env::var("RELAYS").unwrap();
    let relays = serde_json::from_str::<Vec<&str>>(relays.trim()).unwrap();

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
                            let event = match format_blocks(new_blocks) {
                                Ok(event) => event,
                                Err(_) => EventNonSigned {
                                    created_at: unix_timestamp(),
                                    kind: 1,
                                    content: "I seem to be having some trouble, #[0] can you check on me?".to_string(),
                                    tags: vec![vec!["p".to_string(), "04918dfc36c93e7db6cc0d60f37e1522f1c36b64d3f4b424c532d7c595febbc5".to_string()]]
                                }
                            };
                            sender.lock().await.send(event.sign(&keypair)).await;
                        }
                    }
                    Err(_e) => {
                        let now = std::time::SystemTime::now();
                        if now.duration_since(last_error_time).unwrap() > errors_discard_period {
                            let event = EventNonSigned {
                                created_at: unix_timestamp(),
                                kind: 1,
                                content: String::from("I'm unable to reach the API. #[0] help me"),
                                tags: vec![vec!["p".to_string(), "04918dfc36c93e7db6cc0d60f37e1522f1c36b64d3f4b424c532d7c595febbc5".to_string()]]
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
        .name("bitcoin palindrome bot")
        .about("Bot publishing info about palindrome blocks. \n Using https://mempool.space/ API. \n Code at palindromeblock.lol")
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
