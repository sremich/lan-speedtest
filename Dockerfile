# Two build stages, one small runtime image.
#
# VERSION and GIT_SHA come from the release workflow and are surfaced by the
# app at /api/status and in the page footer.

# --- front end ---------------------------------------------------------------
FROM node:22-bookworm-slim AS frontend

WORKDIR /build/frontend
# Dependencies first so a source-only change does not reinstall the engine.
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund

# Every page is a Vite entry point; a missing one fails the build rather than
# silently shipping an image without it.
COPY frontend/tsconfig.json frontend/vite.config.ts ./
COPY frontend/*.html ./
COPY frontend/src ./src
RUN npx tsc --noEmit && npx vite build


# --- backend -----------------------------------------------------------------
FROM rust:1.90-bookworm AS backend

ARG VERSION=0.0.0
ARG GIT_SHA=unknown
ENV APP_VERSION=${VERSION} \
    APP_GIT_SHA=${GIT_SHA}

WORKDIR /build

# Warm the dependency cache against a stub main, so editing our own sources
# does not rebuild the whole tree every time.
COPY backend/Cargo.toml backend/Cargo.lock ./backend/
RUN mkdir -p backend/src \
    && echo 'fn main() {}' > backend/src/main.rs \
    && echo '' > backend/src/lib.rs \
    && echo '0.0.0' > VERSION \
    && printf 'fn main() {}\n' > backend/build.rs \
    && cargo build --release --manifest-path backend/Cargo.toml \
    && rm -rf backend/src backend/build.rs

COPY VERSION ./VERSION
COPY backend/build.rs ./backend/build.rs
COPY backend/src ./backend/src
# Cargo skips rebuilding untouched units; force ours to be rebuilt with the
# real sources and the real version.
RUN touch backend/src/main.rs backend/src/lib.rs \
    && cargo build --release --manifest-path backend/Cargo.toml \
    && strip backend/target/release/lan-speedtest


# --- runtime -----------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

ARG VERSION=0.0.0
ARG GIT_SHA=unknown
ENV APP_VERSION=${VERSION} \
    APP_GIT_SHA=${GIT_SHA} \
    SPEEDTEST_CONFIG=/app/config/speedtest.toml \
    SPEEDTEST_STATIC_DIR=/app/static \
    SPEEDTEST_BIND=0.0.0.0:8080

# ca-certificates is not needed: this service makes no outbound requests.
# curl is for the health check; libcap2-bin provides setcap, used below.
# The group id is pinned to match the user id: `useradd --uid` alone lets the
# distribution pick any free gid, which then silently defeats group-readable
# bind mounts such as the TLS certificate.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl libcap2-bin \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 speedtest \
    && useradd --system --uid 10001 --gid 10001 --no-create-home speedtest

WORKDIR /app
COPY --from=backend /build/backend/target/release/lan-speedtest /usr/local/bin/lan-speedtest
# Lets the unprivileged user bind 443 for TLS. `cap_add` alone is not enough:
# a non-root process needs the capability in its effective set, which is what
# the file capability provides.
RUN setcap 'cap_net_bind_service=+ep' /usr/local/bin/lan-speedtest
COPY --from=frontend /build/frontend/dist /app/static
COPY config/speedtest.toml /app/config/speedtest.toml

# The image redistributes MIT-licensed code (the engine, the whole Rust tree)
# and GPL Debian binaries, all of which require their notices to ship with it.
# Into the static dir so they are also reachable over HTTP — the bundle's own
# banner points at /THIRD-PARTY-NOTICES.md and that link has to resolve.
COPY LICENSE THIRD-PARTY-NOTICES.md /app/static/

# History is written here. Created up front and owned by the runtime user, so a
# bind-mounted volume inherits somewhere writable rather than failing at start.
RUN install -d -o speedtest -g speedtest /app/data

USER speedtest
EXPOSE 8080 443

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1

ENTRYPOINT ["/usr/local/bin/lan-speedtest"]
