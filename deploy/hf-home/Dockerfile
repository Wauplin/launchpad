# syntax=docker/dockerfile:1.7
FROM node:24.14.1-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

FROM rust:1.93.1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang && rm -rf /var/lib/apt/lists/*
WORKDIR /src
ARG LAUNCHPAD_REVISION
ENV LAUNCHPAD_BUILD_COMMIT=${LAUNCHPAD_REVISION}
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY skills ./skills
COPY desktop/src-tauri ./desktop/src-tauri
COPY --from=web /src/web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target cargo build --locked --release -p launchpad && cp target/release/launchpad /launchpad

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv git gh openssh-client ca-certificates tini && rm -rf /var/lib/apt/lists/*
RUN git config --system credential.https://github.com.helper '!gh auth git-credential'
RUN python3 -m venv /opt/venv && /opt/venv/bin/pip install --no-cache-dir aiohttp==3.13.3 tomlkit==0.14.0
RUN useradd --uid 1000 --create-home --shell /bin/bash user
COPY --from=build /launchpad /usr/local/bin/launchpad
COPY deploy/hf-space/gateway/__init__.py deploy/hf-space/gateway/proxy.py /opt/home/gateway/
COPY deploy/hf-home/main.py /opt/home/main.py
ENV PATH=/opt/venv/bin:/usr/local/bin:/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 HOME=/home/user
WORKDIR /opt/home
USER user
EXPOSE 7860
ENTRYPOINT ["/usr/bin/tini", "--", "python", "main.py"]
