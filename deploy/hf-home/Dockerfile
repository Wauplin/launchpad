# syntax=docker/dockerfile:1.7
FROM node:24.14.1-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

FROM rust:1.98.0-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang && rm -rf /var/lib/apt/lists/*
WORKDIR /src
ARG LAUNCHPAD_REVISION
ENV LAUNCHPAD_BUILD_COMMIT=${LAUNCHPAD_REVISION}
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY skills ./skills
COPY --from=web /src/web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target cargo build --locked --release -p launchpad -j 2 && cp target/release/launchpad /launchpad

# Funes (recall, launchpad_recall): built from source at the commit pinned in release.json ("funes"."commit"), with the
# compiler its rust-toolchain.toml names ("funes"."toolchain": keep the image tag below in step). The build layer is
# reused until that commit changes. Release images take the binary CI built instead (build.yml, Dockerfile.release).
FROM debian:bookworm-slim AS funes-pin
COPY deploy/hf-home/release.json /release.json
RUN sed -n 's/.*"commit": *"\([0-9a-f]\{40\}\)".*/\1/p' /release.json > /commit && test -s /commit
FROM rust:1.99.1-bookworm AS funes
RUN apt-get update && apt-get install -y --no-install-recommends cmake protobuf-compiler libprotobuf-dev && rm -rf /var/lib/apt/lists/*
COPY --from=funes-pin /commit /commit
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/funes-target \
    CARGO_TARGET_DIR=/funes-target cargo install --git https://github.com/huggingface/funes --rev "$(cat /commit)" --locked --root /funes -j 2 funes \
    && strip /funes/bin/funes && DO_NOT_TRACK=1 /funes/bin/funes --version

FROM node:24.14.1-bookworm-slim
COPY --from=build /launchpad /usr/local/bin/launchpad
COPY --from=funes /funes/bin/funes /usr/local/bin/funes
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv git openssh-client ca-certificates curl tini && rm -rf /var/lib/apt/lists/*
COPY deploy/hf-home/release.json /opt/home/release.json
COPY deploy/hf-home/install_tools.py /opt/home/install_tools.py
RUN --mount=type=cache,target=/root/.npm python3 /opt/home/install_tools.py
RUN git config --system credential.https://github.com.helper '!gh auth git-credential'
# Tools agents use on a Burst runner (compilers, Rust, pnpm, jq, Chromium…).
COPY deploy/hf-home/agent-tools.sh /opt/home/agent-tools.sh
RUN sh /opt/home/agent-tools.sh
COPY deploy/hf-home/requirements.txt /opt/home/requirements.txt
RUN python3 -m venv /opt/venv && /opt/venv/bin/pip install --no-cache-dir -r /opt/home/requirements.txt && ln -s /opt/venv/bin/hf /usr/local/bin/hf && ln -s /opt/venv/bin/uv /usr/local/bin/uv && ln -s /opt/venv/bin/uvx /usr/local/bin/uvx
RUN userdel node && useradd --uid 1000 --create-home --shell /bin/bash user
# A Burst runner adds its own CA to the system store, so its agents trust the runner proxy (launchpad_core::netcap).
RUN chown user /etc/ssl/certs/ca-certificates.crt
COPY deploy/hf-home/gateway/__init__.py deploy/hf-home/gateway/proxy.py /opt/home/gateway/
COPY deploy/hf-home/main.py deploy/hf-home/onboarding.py deploy/hf-home/setup.html deploy/hf-home/install-runner.sh /opt/home/
ENV PATH=/opt/venv/bin:/usr/local/bin:/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 HOME=/home/user
# Recall: where Home finds funes' model weights (install_tools.py). Home sets HF_HOME, HF_HUB_CACHE, HF_HUB_OFFLINE and
# DO_NOT_TRACK on the funes process itself, never here: Home's own HF login lives in another HF_HOME (main.py).
ENV LAUNCHPAD_FUNES_HF_HOME=/opt/funes/hf
WORKDIR /opt/home
USER user
RUN claude --version && codex --version && opencode --version && gh --version && hf version && uvx --version && monty --version && ruff --version && slackline --version && DO_NOT_TRACK=1 funes --version && test -f /opt/funes/hf/hub/models--BAAI--bge-small-en-v1.5/snapshots/*/model.safetensors && cargo clippy --version && pnpm --version && jq --version && chromium --version
EXPOSE 7860
ENTRYPOINT ["/usr/bin/tini", "--", "python", "main.py"]
