FROM rust:1.65-bullseye

COPY Cargo.toml Cargo.lock /app/
COPY src /app/src

RUN cd app && cargo build --release

ENV RUST_LOG=debug

CMD cd /app && cargo run --release
