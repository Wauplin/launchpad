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
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target cargo build --locked --release -p launchpad -j 2 && cp target/release/launchpad /launchpad

FROM node:24.14.1-bookworm-slim
COPY --from=build /launchpad /usr/local/bin/launchpad
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv git openssh-client ca-certificates curl tini && rm -rf /var/lib/apt/lists/*
COPY deploy/hf-space/release.json /opt/space/release.json
COPY deploy/hf-space/supervisor/install_tools.py /opt/space/install_tools.py
RUN --mount=type=cache,target=/root/.npm python3 /opt/space/install_tools.py
RUN git config --system credential.https://github.com.helper '!gh auth git-credential'
COPY deploy/hf-space/requirements.txt /opt/home/requirements.txt
RUN python3 -m venv /opt/venv && /opt/venv/bin/pip install --no-cache-dir -r /opt/home/requirements.txt && ln -s /opt/venv/bin/hf /usr/local/bin/hf && ln -s /opt/venv/bin/uv /usr/local/bin/uv
RUN userdel node && useradd --uid 1000 --create-home --shell /bin/bash user
COPY deploy/hf-space/gateway/__init__.py deploy/hf-space/gateway/proxy.py /opt/home/gateway/
COPY deploy/hf-home/main.py deploy/hf-home/onboarding.py deploy/hf-home/setup.html deploy/hf-home/install-runner.sh /opt/home/
ENV PATH=/opt/venv/bin:/usr/local/bin:/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 HOME=/home/user
WORKDIR /opt/home
USER user
RUN claude --version && codex --version && opencode --version && gh --version && hf version
EXPOSE 7860
ENTRYPOINT ["/usr/bin/tini", "--", "python", "main.py"]
