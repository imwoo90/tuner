# syntax=docker/dockerfile:1
ARG BUILD_SOURCE=source

FROM rust:1-bookworm AS builder-source
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs build_linter.rs .agent-lint.toml ./
COPY src ./src
COPY _home_defaults ./_home_defaults
RUN cargo build --release --locked && cp target/release/tuner /out-tuner

FROM debian:trixie-slim AS builder-prebuilt
COPY target/release/tuner /out-tuner

FROM builder-${BUILD_SOURCE} AS selected-builder

FROM debian:trixie-slim
ARG TARGETARCH
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git screen procps ffmpeg xvfb chromium \
    fonts-liberation fonts-noto-cjk \
    nodejs npm python3 python3-pip python3-venv tzdata \
    && pip3 install --no-cache-dir --break-system-packages \
       google-genai google-api-python-client google-auth-oauthlib \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash tuner && mkdir -p /opt/tuner/bin

COPY --from=selected-builder /out-tuner /opt/tuner/bin/tuner
COPY _home_defaults /opt/tuner/bin/_home_defaults
COPY scripts/docker-entrypoint.sh /opt/tuner/bin/docker-entrypoint.sh

RUN ARCH="${TARGETARCH:-$(dpkg --print-architecture)}" && \
    case "$ARCH" in \
      arm64|aarch64) CF_ARCH="arm64" ;; \
      *) CF_ARCH="amd64" ;; \
    esac && \
    curl -fsSL "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-${CF_ARCH}" \
      -o /opt/tuner/bin/cloudflared && \
    chmod 0755 /opt/tuner/bin/tuner /opt/tuner/bin/cloudflared /opt/tuner/bin/docker-entrypoint.sh && \
    chown -R tuner:tuner /opt/tuner

USER tuner
WORKDIR /home/tuner

ENV HOME=/home/tuner
ENV PATH="/home/tuner/.tuner/bin:/opt/tuner/bin:/home/tuner/.gemini/antigravity-cli/bin:/home/tuner/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

RUN mkdir -p /home/tuner/.tuner/bin /home/tuner/.local/bin /home/tuner/.gemini && \
    cp /opt/tuner/bin/tuner /home/tuner/.tuner/bin/tuner && \
    cp /opt/tuner/bin/cloudflared /home/tuner/.tuner/bin/cloudflared && \
    cp -r /opt/tuner/bin/_home_defaults /home/tuner/.tuner/bin/_home_defaults

EXPOSE 8743
VOLUME ["/home/tuner/.tuner", "/home/tuner/.gemini"]
ENTRYPOINT ["/opt/tuner/bin/docker-entrypoint.sh"]
