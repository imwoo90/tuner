# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs build_linter.rs .agent-lint.toml ./
COPY src ./src
COPY _home_defaults ./_home_defaults
RUN cargo build --release --locked

FROM debian:trixie-slim
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git screen procps ffmpeg xvfb chromium \
    fonts-liberation fonts-noto-cjk \
    nodejs npm python3 python3-pip python3-venv tzdata \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash tuner
USER tuner
WORKDIR /home/tuner

ENV HOME=/home/tuner
ENV PATH="/home/tuner/.tuner/bin:/home/tuner/.gemini/antigravity-cli/bin:/home/tuner/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

RUN mkdir -p /home/tuner/.tuner/bin /home/tuner/.local/bin /home/tuner/.gemini
COPY --from=builder --chown=tuner:tuner /src/target/release/tuner /home/tuner/.tuner/bin/tuner
COPY --from=builder --chown=tuner:tuner /src/_home_defaults /home/tuner/.tuner/bin/_home_defaults

RUN curl -fsSL https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64 \
    -o /home/tuner/.tuner/bin/cloudflared && chmod +x /home/tuner/.tuner/bin/cloudflared

EXPOSE 8743
VOLUME ["/home/tuner/.tuner", "/home/tuner/.gemini"]
ENTRYPOINT ["/home/tuner/.tuner/bin/tuner"]
