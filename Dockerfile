FROM rust:slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --features server --bin atrium-server

FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="Atrium" \
      org.opencontainers.image.description="GitHub Issues compatible comment backend service" \
      org.opencontainers.image.authors="jihuayu" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="https://github.com/jihuayu/atrium"
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/atrium-server /usr/local/bin/
COPY --from=builder /app/LICENSE /licenses/LICENSE
EXPOSE 3000
CMD ["atrium-server"]
