//! 反代号池 Web 服务入口。
//!
//! 一个进程起两个 HTTP 面：
//! · **下游面**（`GATEWAY_PORT`，默认 8790）：`/v1/*` 给 OpenAI / Anthropic / Responses 客户端；
//! · **面板面**（`PANEL_PORT`，默认 8791）：`/api/*` 管理接口 + 静态面板。
//!
//! 环境变量：
//! | 变量 | 默认 | 说明 |
//! |---|---|---|
//! | `GATEWAY_DATA_DIR` | `/data` | 配置、SQLite、主密钥所在目录 |
//! | `GATEWAY_PORT` / `GATEWAY_BIND` | `8790` / `0.0.0.0` | 下游监听 |
//! | `PANEL_PORT` / `PANEL_BIND` | `8791` / `0.0.0.0` | 面板监听 |
//! | `PANEL_USERNAME` / `PANEL_PASSWORD` | `admin` / **必填** | 面板登录 |
//! | `PANEL_SECRET` | 随机 | 会话签名盐（显式配置可跨重启保持登录） |
//! | `PANEL_ALLOW_OPEN` | 关 | `1` = 允许无密码开放（仅本地调试） |
//! | `GATEWAY_MASTER_KEY` | 文件 | base64 的 32 字节凭据主密钥 |
//! | `WEB_DIST` | `./web` | 前端静态资源目录（不存在则只提供 API） |
//! | `TZ` | `UTC` | 本地时区（影响"今日"与每日动作） |

mod gateway;
mod panel;
mod secrets;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use panel::{auth::AuthConfig, PanelCtx};

#[tokio::main]
async fn main() {
    // 容器 HEALTHCHECK 用：探一下面板端口就退出，不起服务、不碰数据目录。
    if std::env::args().any(|arg| arg == "--healthcheck") {
        std::process::exit(healthcheck().await);
    }
    if let Err(error) = run().await {
        eprintln!("[fatal] {error}");
        std::process::exit(1);
    }
}

/// 探活：`/api/auth/session` 返回 401 也算活着（说明 HTTP 服务在响应）。
async fn healthcheck() -> i32 {
    let port = env_u16("PANEL_PORT").unwrap_or(8791);
    let url = format!("http://127.0.0.1:{port}/api/auth/session");
    match reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("[healthcheck] {url} 不通：{error}");
            1
        }
    }
}

async fn run() -> Result<(), String> {
    // 面板鉴权先校验：宁可起不来，也不要"部署完发现管理面裸奔"。
    let auth = AuthConfig::from_env()?;

    let data_dir = data_dir();
    eprintln!("[boot] 数据目录 {}", data_dir.display());
    let gateway_dir = data_dir.join("gateway");
    let state = gateway::setup(&gateway_dir).map_err(|error| format!("网关初始化失败：{error}"))?;
    eprintln!("[boot] 时区偏移 {} 秒（TZ={}）", 
        gateway::timeutil::local_utc_offset_secs(),
        std::env::var("TZ").unwrap_or_else(|_| "未设置(按 UTC)".into()));

    let web_dir = web_dir();
    match &web_dir {
        Some(dir) => eprintln!("[boot] 面板静态资源 {}", dir.display()),
        None => eprintln!("[boot] 未找到面板静态资源（只提供 API）"),
    }

    // 面板面
    let panel_port = env_u16("PANEL_PORT").unwrap_or(8791);
    let panel_bind = env_str("PANEL_BIND").unwrap_or_else(|| "0.0.0.0".into());
    let panel_ctx = PanelCtx {
        gateway: Arc::clone(&state),
        auth,
    };
    let panel_app = panel::full_router(panel_ctx, web_dir);
    let panel_listener = bind(&panel_bind, panel_port).await?;
    eprintln!(
        "[panel] 面板已监听 http://{}",
        panel_listener
            .local_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| format!("{panel_bind}:{panel_port}"))
    );

    // 下游面：网关自身的 `server::run` 已经会按 GATEWAY_PORT/GATEWAY_BIND 起服务，
    // 这里只等它和面板一起跑（任一退出即整体退出）。
    let panel_server = axum::serve(panel_listener, panel_app);

    tokio::select! {
        result = panel_server => {
            if let Err(error) = result {
                return Err(format!("面板服务异常退出：{error}"));
            }
        }
        _ = tokio::signal::ctrl_c() => {
            eprintln!("[boot] 收到退出信号，正在关闭");
        }
    }
    Ok(())
}

/// 数据目录：`GATEWAY_DATA_DIR` → 容器内 `/data` → 当前目录下的 `./data`。
fn data_dir() -> PathBuf {
    if let Some(dir) = env_str("GATEWAY_DATA_DIR") {
        return PathBuf::from(dir);
    }
    // 容器内的约定路径；本地直跑时用相对目录，免得去写宿主根目录。
    if cfg!(unix) && Path::new("/data").is_dir() {
        return PathBuf::from("/data");
    }
    PathBuf::from("./data")
}

/// 前端静态资源目录：`WEB_DIST`，否则找 `./web` / `./dist`。
fn web_dir() -> Option<PathBuf> {
    if let Some(dir) = env_str("WEB_DIST") {
        let path = PathBuf::from(dir);
        return path.join("index.html").exists().then_some(path);
    }
    ["web", "dist"]
        .iter()
        .map(PathBuf::from)
        .find(|path| path.join("index.html").exists())
}

fn env_str(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_u16(name: &str) -> Option<u16> {
    env_str(name)?.parse().ok()
}

async fn bind(host: &str, port: u16) -> Result<tokio::net::TcpListener, String> {
    tokio::net::TcpListener::bind((host, port))
        .await
        .map_err(|error| format!("面板监听 {host}:{port} 失败：{error}"))
}
