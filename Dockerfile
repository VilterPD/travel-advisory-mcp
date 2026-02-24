FROM rust:1.85-bookworm AS build
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
WORKDIR /app
ENV RUST_LOG=info

COPY --from=build /app/target/release/travel-advisory-mcp /usr/local/bin/travel-advisory-mcp

EXPOSE 3000
CMD ["travel-advisory-mcp"]
