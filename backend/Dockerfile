FROM rust:1.99-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/video-editor /usr/local/bin/video-editor

ENV BIND_ADDR=0.0.0.0:3000 \
    DATABASE_URL=sqlite:///data/video_editor.db \
    STORAGE_ROOT=/data/storage \
    FFPROBE_BIN=ffprobe \
    FFMPEG_BIN=ffmpeg

RUN mkdir -p /data/storage
VOLUME ["/data"]
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/video-editor"]

