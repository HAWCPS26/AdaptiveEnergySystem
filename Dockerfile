# We utilize a singular dockerfile for all rust applications to save on compile time costs
FROM rust:bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY tickgen ./tickgen
COPY example_subscriber ./example_subscriber
COPY consumer ./consumer
RUN cargo build --release --workspace

FROM debian:bookworm-slim
RUN useradd --create-home sim
COPY --from=builder /app/target/release/tickgen /usr/local/bin/tickgen
COPY --from=builder /app/target/release/example_subscriber /usr/local/bin/example_subscriber
COPY --from=builder /app/target/release/deferrable_load /usr/local/bin/deferrable_load
USER sim
WORKDIR /home/sim
CMD ["/usr/local/bin/tickgen"]
