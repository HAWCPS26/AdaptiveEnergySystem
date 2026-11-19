FROM rust:bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY tickgen ./tickgen
COPY example_subscriber ./example_subscriber
COPY consumer ./consumer
RUN cargo build --release --package deferrable_load

FROM debian:bookworm-slim
RUN useradd --create-home deferrable
COPY --from=builder /app/target/release/deferrable_load /usr/local/bin/deferrable_load
USER deferrable
WORKDIR /home/deferrable
EXPOSE 4842
ENTRYPOINT ["/usr/local/bin/deferrable_load"]
