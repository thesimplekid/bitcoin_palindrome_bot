FROM rust:1.65-bullseye

COPY Cargo.toml Cargo.lock /app/
COPY src /app/src
COPY .env /app/

RUN cd app && cargo build --release

CMD cd /app && cargo run --release
