FROM rust:slim AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml ./
# Copy Cargo.lock if it exists (run `cargo generate-lockfile` first if missing)
COPY Cargo.lock* ./
# Cache dependencies by building a dummy main first
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src
COPY src ./src
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/scrobgoblin /app/scrobgoblin
CMD ["/app/scrobgoblin"]
