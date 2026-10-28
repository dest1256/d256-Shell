# syntax=docker/dockerfile:1
FROM node:22.23.4-bookworm@sha256:63538dfe497cc29dfafdda25ada09b46db68a3f4cc205abf1f5d6bf0913f3384 AS node

FROM rust:1.99.1-bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      clang lld llvm pkg-config git openssh-client sudo ca-certificates curl \
      # Playwright Chromium runtime deps
      libnss3 libnspr4 libatk1.0-0 libatk-bridge2.0-0 libcups2 libdrm2 libxkbcommon0 \
      libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libgbm1 libasound2 libpango-1.0-0 \
      libcairo2 libx11-6 libxcb1 libxext6 fonts-liberation \
    && rm -rf /var/lib/apt/lists/*

COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx

# Non-root user (no sudo rights granted)
RUN useradd -m -s /bin/bash -u 1000 dev \
 && mkdir -p /build /home/dev/.cargo /home/dev/.cache/cargo-xwin /home/dev/.cache/ms-playwright /workspaces/shellport/ui/node_modules && chown -R dev:dev /build /home/dev /workspaces

# Tools are installed outside CARGO_HOME so the cargo volume never shadows them
ENV CARGO_INSTALL_ROOT=/opt/tools
RUN rustup target add x86_64-pc-windows-msvc \
 && cargo install --locked cargo-xwin tauri-cli cargo-audit cargo-deny \
 && cargo install --locked cargo-about --features cli \
 && rm -rf /usr/local/cargo/registry

# nasm: aws-lc-sys (crypto backend of russh) assembles with NASM when targeting Windows
RUN apt-get update && apt-get install -y --no-install-recommends zip unzip nasm \
 && rm -rf /var/lib/apt/lists/* \
 && rustup component add rustfmt clippy

ENV PATH=/opt/tools/bin:$PATH \
    CARGO_HOME=/home/dev/.cargo \
    CARGO_TARGET_DIR=/build/target \
    XWIN_CACHE_DIR=/home/dev/.cache/cargo-xwin \
    PLAYWRIGHT_BROWSERS_PATH=/home/dev/.cache/ms-playwright
# XWIN_ACCEPT_LICENSE is deliberately NOT set here: set it only after the user accepts the Microsoft licence.
USER dev
WORKDIR /workspaces/shellport
