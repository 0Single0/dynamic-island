# =============================================================================
# 反代号池 Web 服务：单容器（Rust 网关 + 面板静态资源）
#
# 三段构建：
#   1. web     —— 用 pnpm 构建面板静态资源
#   2. rust    —— 编译 gateway-server（rusqlite 走 bundled，静态编译 SQLite）
#   3. runtime —— 只带二进制 + 静态资源 + CA 证书的最小镜像
#
# 构建参数：
#   CARGO_MIRROR   crates.io 镜像（如 sparse+https://rsproxy.cn/index/）。
#                  留空则用官方 crates.io；国内网络建议显式指定。
# =============================================================================

# ---------- 1. 前端 ----------
FROM node:24-bookworm-slim AS web

ENV PNPM_HOME=/pnpm
ENV PATH=$PNPM_HOME:$PATH

WORKDIR /app

# 先只拷清单：依赖没变时这层缓存能复用，改代码不会重新装包。
#
# `packageManager` 字段把 pnpm 钉在 lockfile 的生成版本上：
# corepack 默认会去装**最新版** pnpm，而新版对旧 lockfile 可能直接拒绝
# （踩过：pnpm 12 拿 pnpm 10 写的 lockfile 报 ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION）。
COPY package.json pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile

COPY index.html vite.config.ts tsconfig.json tsconfig.node.json ./
COPY src ./src
RUN pnpm build


# ---------- 2. Rust 网关 ----------
FROM rust:1.96-bookworm AS rust

ARG CARGO_MIRROR=""

WORKDIR /src

# 镜像配置必须在 cargo 之前落盘，否则第一次解析就打到官方源上。
RUN if [ -n "$CARGO_MIRROR" ]; then \
      mkdir -p /usr/local/cargo && \
      printf '[source.crates-io]\nreplace-with = "mirror"\n\n[source.mirror]\nregistry = "%s"\n\n[net]\nretry = 5\n' "$CARGO_MIRROR" \
        > /usr/local/cargo/config.toml; \
    fi

COPY server/Cargo.toml server/Cargo.lock ./
COPY server/src ./src

# 依赖与源码一起编译：项目不大，不值得为缓存拆 dummy target。
RUN cargo build --release --locked \
    && strip target/release/gateway-server


# ---------- 3. 运行时 ----------
FROM debian:bookworm-slim AS runtime

# 故意不装任何包：
# · TLS 根证书 —— reqwest 走 rustls + webpki-roots，根证书是编进二进制的；
# · tzdata    —— timeutil 自己解析 TZ（内置常用区域表），不读 /usr/share/zoneinfo。
# 于是运行层不依赖 Debian 镜像源，构建更稳、镜像更小。
# 若上游需要企业自签 CA，再在扩展镜像里装 ca-certificates 或挂载证书。

ENV TZ=Asia/Shanghai \
    GATEWAY_DATA_DIR=/data \
    GATEWAY_PORT=8790 \
    GATEWAY_BIND=0.0.0.0 \
    PANEL_PORT=8791 \
    PANEL_BIND=0.0.0.0 \
    WEB_DIST=/app/web

WORKDIR /app
COPY --from=rust /src/target/release/gateway-server /usr/local/bin/gateway-server
COPY --from=web /app/dist /app/web

RUN mkdir -p /data && useradd --uid 10001 --home-dir /app --shell /usr/sbin/nologin gateway \
    && chown -R gateway:gateway /data /app
USER gateway

# 8790 = 下游反代面（/v1/*），8791 = 面板面（/api/* + 静态页）
EXPOSE 8790 8791

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/gateway-server", "--healthcheck"]

ENTRYPOINT ["/usr/local/bin/gateway-server"]
