# stx-server: build the engine HTTP service and run it in a slim image.
FROM rust:1-slim-bookworm AS builder
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
RUN cargo build --release -p stx-server

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/stx-server /usr/local/bin/stx-server
ENV PORT=8080
EXPOSE 8080
CMD ["stx-server"]
