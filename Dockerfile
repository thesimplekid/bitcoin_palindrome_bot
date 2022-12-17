FROM rust:1.65-bullseye

COPY Cargo.toml Cargo.lock /app/
COPY src /app/src

RUN cd app && cargo build --release

ENV RUST_LOG=debug \ 
        RELAYS="['wss://nostr-pub.wellorder.net', 'wss://relay.nostr.info', 'wss://relay.damus.io', 'wss://nostr.delo.software', 'wss://nostr.zaprite.io', 'wss://nostr.zebedee.cloud']"

CMD cd /app && cargo run --release
