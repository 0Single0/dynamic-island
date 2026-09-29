//! CDP（Chrome DevTools Protocol）客户端。
//!
//! 只连接本机回环的调试端口，对网易云播放器页面执行白名单表达式：
//! 读取播放器快照、点击控制按钮。不提供“运行任意脚本”的通用能力。

use serde_json::Value;

/// `/json/list` 中选中的页面目标。
#[derive(Debug, Clone)]
pub struct CdpTarget {
    pub title: String,
    pub url: String,
    pub ws_url: String,
}

/// 带超时的共享 HTTP 客户端：本机端口可能被防火墙丢弃（超时而非拒绝），
/// 不设超时会导致启动流程永久挂起。
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(800))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .expect("failed to build http client")
}

/// 在调试端口上寻找网易云播放器页面。
pub async fn find_target(port: u16) -> anyhow::Result<Option<CdpTarget>> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let targets: Vec<Value> = http_client().get(&url).send().await?.json().await?;

    Ok(targets
        .iter()
        .filter(|t| t["type"].as_str() == Some("page"))
        .find(|t| {
            t["url"].as_str().unwrap_or("").starts_with("orpheus://")
                || t["title"].as_str() == Some("网易云音乐")
        })
        .and_then(|t| {
            Some(CdpTarget {
                title: t["title"].as_str()?.to_string(),
                url: t["url"].as_str()?.to_string(),
                ws_url: t["webSocketDebuggerUrl"].as_str()?.to_string(),
            })
        }))
}

/// 播放器快照（`read_player_snapshot` 表达式的返回结构）。
/// 页面表达式返回 camelCase 键，必须 rename 对齐，否则全部落 default 值。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayerSnapshot {
    pub available: bool,
    pub reason: Option<String>,
    pub title: String,
    pub artist: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub cover_art: String,
    pub is_playing: bool,
}

/// 读取播放器快照的页面表达式（只读 #page_pc_mini_bar 局部）。
pub const READ_SNAPSHOT_EXPR: &str = r#"
(() => {
  const bar = document.querySelector('#page_pc_mini_bar');
  if (!bar) return { available: false, reason: 'player-not-found' };

  const title = bar.querySelector('.vinly-title')?.textContent?.trim() ?? '';
  const timeText = document.querySelector('.curtime-thumb, [class*="curtime-thumb"], [class*="TimeWarpper"]')?.textContent
    ?? bar.innerText
    ?? '';
  const time = timeText.match(/(\d{1,2}):(\d{2})\s*\/\s*(\d{1,2}):(\d{2})/);
  const positionMs = time ? (+time[1] * 60 + +time[2]) * 1000 : 0;
  const durationMs = time ? (+time[3] * 60 + +time[4]) * 1000 : 0;
  const playButton = bar.querySelector('#btn_pc_minibar_play');
  // 双保险：aria-label 与 cmd-icon-pause/play class（客户端更新可能丢 aria）
  const pauseIcon = playButton?.querySelector('[aria-label="pause"], .cmd-icon-pause');
  const playIcon = playButton?.querySelector('[aria-label="play"], .cmd-icon-play');

  // 歌手：迷你条固定结构为 .main-title 下与 .vinly-title 并列的 span.author
  const artist = bar.querySelector('.main-title .author')?.textContent?.trim()
    ?? bar.querySelector('.author')?.textContent?.trim()
    ?? '';

  // 封面：底部播放条黑胶封面（.miniVinylWrapper img，orpheus://cache 源）
  const coverArt = document.querySelector('.miniVinylWrapper img')?.src
    ?? bar.querySelector('img')?.src
    ?? '';

  // 迷你条没有时间文本（客户端改版后时间只在底部主播放条），
  // 进度从底部条的进度条宽度推算；条不可见时退回 audio 元素 currentTime
  let pos = positionMs;
  let dur = durationMs;
  if (dur === 0) {
    const barEl = document.querySelector('#simple-bar,.bar-wrapper,.progress-wrapper');
    const audio = document.querySelector('audio');
    if (audio && audio.duration && isFinite(audio.duration)) {
      dur = Math.round(audio.duration * 1000);
      pos = Math.round(audio.currentTime * 1000);
    } else if (barEl) {
      const fill = barEl.querySelector('[class*=progress] i,[class*=progress] b,[class*=cur]');
      const wrap = barEl.querySelector('[class*=progress]');
      if (fill && wrap && wrap.offsetWidth > 0) {
        const ratio = Math.min(1, fill.offsetWidth / wrap.offsetWidth);
        // ratio 有了但时长仍未知，只能给比例；时长交由 playingList 匹配补充
        pos = Math.round(ratio * 1000);
      }
    }
  }

  return {
    available: true,
    title,
    artist,
    positionMs: pos,
    durationMs: dur,
    coverArt,
    isPlaying: Boolean(pauseIcon) && !Boolean(playIcon)
  };
})()
"#;

/// 点击控制按钮的页面表达式。
fn click_expr(selector: &str) -> String {
    format!(
        r#"(() => {{ const el = document.querySelector('{selector}'); if (el) {{ el.click(); return true; }} return false; }})()"#
    )
}

/// 拖动网易云原始进度滑轨。页面目前使用 `.slider-vinyl`，但 class hash
/// 会随版本变化，因此同时匹配稳定前缀和生成类名片段。
fn seek_expr(position_ms: u64, duration_ms: u64) -> String {
    format!(
        r#"(() => {{
  const slider = [...document.querySelectorAll('.slider-vinyl, [class*="slider-vinyl"], [class*="StyledSliderContainer"]')]
    .find((el) => el.getBoundingClientRect().width > 50);
  if (!slider || {duration_ms} === 0) return false;
  const rect = slider.getBoundingClientRect();
  const ratio = Math.max(0, Math.min(1, {position_ms} / {duration_ms}));
  const clientX = rect.left + rect.width * ratio;
  const clientY = rect.top + rect.height / 2;
  const mouse = (type) => slider.dispatchEvent(new MouseEvent(type, {{
    bubbles: true, cancelable: true, view: window, clientX, clientY, buttons: type === 'mouseup' ? 0 : 1
  }}));
  const pointer = (type) => {{
    if (window.PointerEvent) slider.dispatchEvent(new PointerEvent(type, {{
      bubbles: true, cancelable: true, view: window, clientX, clientY, buttons: type === 'pointerup' ? 0 : 1,
      pointerId: 1, pointerType: 'mouse', isPrimary: true
    }}));
  }};
  pointer('pointerdown'); mouse('mousedown'); pointer('pointermove'); mouse('mousemove');
  pointer('pointerup'); mouse('mouseup'); mouse('click');
  return true;
}})()"#
    )
}

const SEL_PREV: &str = r#"[data-testid="tid_playbar_prev_btn"]"#;
const SEL_PLAY: &str = "#btn_pc_minibar_play";
const SEL_NEXT: &str = r#"[data-testid="tid_playbar_next_btn"]"#;

/// 已连接的 CDP 页面，顺序执行一次表达式。
pub struct CdpSession {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
}

impl CdpSession {
    pub async fn connect(ws_url: &str) -> anyhow::Result<Self> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
        Ok(Self { ws, next_id: 1 })
    }

    /// 执行一个表达式并取回 JSON 结果（Runtime.evaluate，返回 byArgument value）。
    pub async fn evaluate(&mut self, expression: &str) -> anyhow::Result<Value> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let id = self.next_id;
        self.next_id += 1;
        let request = serde_json::json!({
            "id": id,
            "method": "Runtime.evaluate",
            "params": {
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": false,
            }
        });
        self.ws
            .send(Message::Text(request.to_string().into()))
            .await?;

        // 读到对应 id 的响应为止；忽略事件与其他响应
        loop {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(3), self.ws.next())
                .await
                .map_err(|_| anyhow::anyhow!("cdp evaluate timeout"))?
                .ok_or_else(|| anyhow::anyhow!("cdp ws closed"))??;
            let Message::Text(text) = msg else { continue };
            let value: Value = serde_json::from_str(&text)?;
            if value["id"].as_u64() == Some(id) {
                if let Some(err) = value["error"].as_str() {
                    anyhow::bail!("cdp error: {err}");
                }
                return Ok(value["result"]["result"]["value"].clone());
            }
        }
    }

    pub async fn read_snapshot(&mut self) -> anyhow::Result<PlayerSnapshot> {
        let value = self.evaluate(READ_SNAPSHOT_EXPR).await?;
        Ok(serde_json::from_value(value).unwrap_or_default())
    }

    /// 播放控制：previous / toggle / next。返回页面是否找到按钮。
    pub async fn control(&mut self, action: &str) -> anyhow::Result<bool> {
        let selector = match action {
            "previous" => SEL_PREV,
            "toggle" => SEL_PLAY,
            "next" => SEL_NEXT,
            other => anyhow::bail!("unknown control action: {other}"),
        };
        let value = self.evaluate(&click_expr(selector)).await?;
        Ok(value.as_bool().unwrap_or(false))
    }

    /// 跳转到指定毫秒位置。进度条属于网易云自身，必须让页面原始滑轨
    /// 接收完整的指针事件序列，而不是只更新我们自己的展示进度。
    pub async fn seek(&mut self, position_ms: u64, duration_ms: u64) -> anyhow::Result<bool> {
        if duration_ms == 0 {
            return Ok(false);
        }
        let value = self
            .evaluate(&seek_expr(position_ms.min(duration_ms), duration_ms))
            .await?;
        Ok(value.as_bool().unwrap_or(false))
    }
}
