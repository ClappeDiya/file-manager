# syntax=docker/dockerfile:1.7
# UFOP headless backup server (multi-arch: linux/amd64, linux/arm64).
#
#   docker build -f docker/ufop-server.Dockerfile -t ufop-server .
#   (behind a TLS-intercepting proxy add: --network host --secret id=ca,src=ca.crt
#    --build-arg https_proxy=…; see docker/with-ca.sh)
#   scripts/docker-build.sh --platforms linux/amd64,linux/arm64 --push   # one manifest list on GHCR
#
# Multi-arch: the web UI is built once on the build machine's platform
# (BUILDPLATFORM; its output is architecture-independent), the server binary
# natively for each TARGETPLATFORM (QEMU emulation for a foreign arch, or a
# native node of the buildx builder). Natively compiling under QEMU is the
# robust choice here: ufop-server links GTK/WebKitGTK through the Tauri
# library, and cross-linking those needs a full multiarch sysroot. The cost
# is time: an emulated arm64 build is several times slower than a native
# one; append a native arm64 node to the builder to avoid it (see
# docs/release-checklist.md, "Docker images").
#
#   docker run -d -p 8200:8200 -v ufop-data:/data -v /home:/source:ro \
#     -e UFOP_SERVER_PASSWORD=change-me ufop-server
#
# Settings come from environment variables (see ufop-server --help):
# UFOP_SERVER_PORT, UFOP_SERVER_ALLOWED_HOSTNAMES, UFOP_SERVER_PREAUTH_TOKENS,
# UFOP_SERVER_CORS_ORIGINS, UFOP_SETTINGS_ENCRYPTION_KEY, UFOP_ALLOWED_BACKENDS…

# Official images; override the registry when Docker Hub is rate-limited:
#   --build-arg REGISTRY=public.ecr.aws/docker/library
ARG REGISTRY=docker.io/library

# ── Web UI (once, on the build machine's platform) ──
FROM --platform=$BUILDPLATFORM ${REGISTRY}/node:22-bookworm-slim AS web
WORKDIR /src
RUN --mount=type=secret,id=ca --mount=type=bind,source=docker/with-ca.sh,target=/with-ca.sh sh /with-ca.sh corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml .npmrc .pnpmfile.cjs ./
COPY packages ./packages
COPY admin/package.json ./admin/package.json
COPY marketing/package.json ./marketing/package.json
RUN --mount=type=secret,id=ca --mount=type=bind,source=docker/with-ca.sh,target=/with-ca.sh sh /with-ca.sh sh -c 'corepack prepare --activate && pnpm install --frozen-lockfile --ignore-scripts'
COPY . .
RUN --mount=type=secret,id=ca --mount=type=bind,source=docker/with-ca.sh,target=/with-ca.sh sh /with-ca.sh pnpm build

# ── Server binary (per target platform) ──
FROM ${REGISTRY}/ubuntu:24.04 AS server
# Set by BuildKit for each --platform (amd64, arm64); plain `docker build` sets it too.
ARG TARGETARCH
ENV DEBIAN_FRONTEND=noninteractive PATH=/root/.cargo/bin:$PATH
RUN --mount=type=secret,id=ca --mount=type=bind,source=docker/with-ca.sh,target=/with-ca.sh sh /with-ca.sh sh -c 'apt-get update && apt-get install -y --no-install-recommends \
      build-essential curl ca-certificates pkg-config libssl-dev \
      libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev \
    && rm -rf /var/lib/apt/lists/*'
RUN --mount=type=secret,id=ca --mount=type=bind,source=docker/with-ca.sh,target=/with-ca.sh sh /with-ca.sh sh -c 'case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
        amd64) arch=x86_64 ;; arm64) arch=aarch64 ;; *) echo "unsupported TARGETARCH ${TARGETARCH}" >&2; exit 1 ;; esac \
      && curl -fsSLo /tmp/rustup-init "https://static.rust-lang.org/rustup/dist/${arch}-unknown-linux-gnu/rustup-init" \
      && chmod +x /tmp/rustup-init && /tmp/rustup-init -y --profile minimal && rm /tmp/rustup-init && cargo --version'
WORKDIR /src
COPY crates ./crates
COPY src-tauri ./src-tauri
# tauri::generate_context! reads the frontend dist path at compile time.
COPY --from=web /src/dist ./dist
# Registry and target caches are kept per architecture between builds.
RUN --mount=type=secret,id=ca --mount=type=bind,source=docker/with-ca.sh,target=/with-ca.sh \
    --mount=type=cache,id=ufop-cargo-registry-${TARGETARCH},target=/root/.cargo/registry \
    --mount=type=cache,id=ufop-server-target-${TARGETARCH},target=/src/src-tauri/target \
    sh /with-ca.sh sh -c 'cd src-tauri && cargo build --release --locked --bin ufop-server && cp target/release/ufop-server /usr/local/bin/ufop-server'

# ── Runtime ──
FROM ${REGISTRY}/ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN --mount=type=secret,id=ca --mount=type=bind,source=docker/with-ca.sh,target=/with-ca.sh sh /with-ca.sh sh -c 'apt-get update && apt-get install -y --no-install-recommends \
      libwebkit2gtk-4.1-0 libgtk-3-0t64 libayatana-appindicator3-1 libssl3t64 ca-certificates tini par2 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /data ufop && mkdir -p /data /source && chown ufop /data \
    && rm -f /etc/apt/apt.conf.d/99extra-ca'
COPY --from=server /usr/local/bin/ufop-server /usr/local/bin/ufop-server
COPY --from=web /src/dist /app/web
COPY docker/entrypoint.sh /usr/local/bin/ufop-entrypoint
USER ufop
ENV UFOP_SERVER_DATADIR=/data \
    UFOP_SERVER_WEBROOT=/app/web \
    UFOP_SERVER_INTERFACE=any \
    UFOP_SERVER_PORT=8200
VOLUME ["/data"]
EXPOSE 8200
HEALTHCHECK --interval=30s --timeout=5s CMD ["/bin/sh", "-c", "exec 3<>/dev/tcp/127.0.0.1/${UFOP_SERVER_PORT}"]
ENTRYPOINT ["tini", "--", "ufop-entrypoint"]
