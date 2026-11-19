FROM rust:bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY tickgen ./tickgen
COPY example_subscriber ./example_subscriber
COPY consumer ./consumer
RUN cargo build --release --package tickgen

FROM debian:bookworm-slim
RUN useradd --create-home tickgen
COPY --from=builder /app/target/release/tickgen /usr/local/bin/tickgen
USER tickgen
WORKDIR /home/tickgen
EXPOSE 4840
ENTRYPOINT ["/usr/local/bin/tickgen"]
