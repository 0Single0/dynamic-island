//! 网易云启动器：检测进程/端口、以 CDP 参数启动、等待页面就绪。
//!
//! 规则（与同步方案一致）：普通模式运行的网易云绝不自动结束，
//! 必须由用户确认后先退出，再由本工具重新启动。

pub const DEFAULT_PORT: u16 = 9222;

/// 探测云音乐可执行文件路径：环境变量覆盖 > 常见安装位置。
pub fn detect_cloudmusic_path() -> Option<std::path::PathBuf> {
    if let Ok(custom) = std::env::var("DYNAMIC_ISLAND_CLOUDMUSIC") {
        let p = std::path::PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }
    let candidates = [
        r"D:\CloudMusic\cloudmusic.exe",
        r"C:\Program Files (x86)\Netease\CloudMusic\cloudmusic.exe",
        r"C:\Program Files\Netease\CloudMusic\cloudmusic.exe",
    ];
    candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
}

/// cloudmusic.exe 是否正在运行。
pub fn is_cloudmusic_running() -> bool {
    // 用 tasklist 检查，避免引入 sysinfo 这类重依赖
    let Ok(output) = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq cloudmusic.exe", "/FO", "CSV", "/NH"])
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.contains("cloudmusic.exe")
}

/// 端口上是否已有网易云 CDP 页面（防止把其他 Chromium 程序误当网易云）。
pub async fn cdp_ready(port: u16) -> bool {
    super::cdp::find_target(port)
        .await
        .map(|t| t.is_some())
        .unwrap_or(false)
}

#[derive(Debug)]
pub enum StartOutcome {
    /// 端口已就绪（网易云本就带 CDP 运行，或刚由本工具启动）。
    Ready,
    /// 网易云在运行但没有 CDP，需要用户确认后由前端调用 restart_with_sync。
    NeedsRestart,
    /// 找不到 cloudmusic.exe。
    NotFound,
    /// 启动后 15 秒内端口未就绪。
    Timeout,
}

/// 启动或连接网易云。见 `StartOutcome`。
pub async fn start_or_connect() -> StartOutcome {
    if cdp_ready(DEFAULT_PORT).await {
        return StartOutcome::Ready;
    }
    if is_cloudmusic_running() {
        return StartOutcome::NeedsRestart;
    }
    let Some(path) = detect_cloudmusic_path() else {
        return StartOutcome::NotFound;
    };
    launch_with_cdp(&path);
    let outcome = wait_for_cdp(DEFAULT_PORT, std::time::Duration::from_secs(15)).await;
    match outcome {
        // 进程在但调试端口没开：多半是客户端自带 updater 抢先重启了实例并丢掉参数。
        // 唯一出路仍是重启，回到 NeedsRestart 让前端重新给出确认按钮。
        StartOutcome::Timeout if is_cloudmusic_running() => StartOutcome::NeedsRestart,
        other => other,
    }
}

/// 用户已确认重启：此时才允许结束网易云进程并以 CDP 参数重新启动。
pub async fn restart_with_sync() -> StartOutcome {
    if !is_cloudmusic_running() {
        return start_or_connect().await;
    }

    // taskkill 不带 /F：先礼貌请求退出；等 3 秒仍在才强制。
    let _ = std::process::Command::new("taskkill")
        .args(["/IM", "cloudmusic.exe"])
        .status();
    for _ in 0..30 {
        if !is_cloudmusic_running() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    if is_cloudmusic_running() {
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", "cloudmusic.exe", "/F"])
            .status();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    start_or_connect().await
}

fn launch_with_cdp(path: &std::path::Path) {
    let _ = std::process::Command::new(path)
        .arg(format!("--remote-debugging-port={DEFAULT_PORT}"))
        .spawn();
}

async fn wait_for_cdp(port: u16, budget: std::time::Duration) -> StartOutcome {
    let deadline = tokio::time::Instant::now() + budget;
    while tokio::time::Instant::now() < deadline {
        if cdp_ready(port).await {
            return StartOutcome::Ready;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    StartOutcome::Timeout
}
