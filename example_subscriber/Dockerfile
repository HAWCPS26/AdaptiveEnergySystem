FROM rust:bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY tickgen ./tickgen
COPY example_subscriber ./example_subscriber
COPY consumer ./consumer
RUN cargo build --release --package example_subscriber

FROM debian:bookworm-slim
RUN useradd --create-home subscriber
COPY --from=builder /app/target/release/example_subscriber /usr/local/bin/example_subscriber
USER subscriber
WORKDIR /home/subscriber
ENTRYPOINT ["/usr/local/bin/example_subscriber"]
