//! Real search and bounded public-page retrieval. MockSearchProvider is never used here.
use super::super::{native_dialog, search::SearchResult, tools::ToolExecutionResult};
use futures_util::StreamExt;
use reqwest::Url;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, net::IpAddr, time::Duration};
use tauri::Manager;

const BODY_LIMIT: usize = 512 * 1024;
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchConfig {
    #[serde(default)]
    pub provider: SearchProvider,
    #[serde(default)]
    pub endpoint: String,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SearchProvider {
    #[default]
    Unconfigured,
    BingPublic,
    Searxng,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    pub query: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchArgs {
    pub url: String,
}

fn config_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("agent-search.json"))
}
pub fn read_config(app: &tauri::AppHandle) -> Result<SearchConfig, String> {
    let path = config_path(app)?;
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SearchConfig::default()),
        Err(e) => Err(e.to_string()),
    }
}
#[tauri::command]
pub fn agent_get_search_config(app: tauri::AppHandle) -> Result<SearchConfig, String> {
    read_config(&app)
}
#[tauri::command]
pub fn agent_save_search_config(
    app: tauri::AppHandle,
    input: SearchConfig,
) -> Result<SearchConfig, String> {
    use std::io::Write;
    if input.provider == SearchProvider::Searxng {
        validate_url(&input.endpoint)?;
    }
    let path = config_path(&app)?;
    let parent = path.parent().ok_or("设置目录不可用")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(&input).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(&path).map_err(|e| e.error.to_string())?;
    Ok(input)
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let a = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || a[0] == 0
                || a[0] >= 224
                || a[0] == 127
                || (a[0] == 100 && (64..=127).contains(&a[1]))
                || (a[0] == 192 && a[1] == 0 && a[2] == 0)
                || (a[0] == 192 && a[1] == 0 && a[2] == 2)
                || (a[0] == 198 && (a[1] == 18 || a[1] == 19 || a[1] == 51))
                || (a[0] == 203 && a[1] == 0 && a[2] == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                is_public(IpAddr::V4(v4))
            } else {
                let s = ip.segments();
                // Only global-unicast space; reject documentation and transition mechanisms.
                (s[0] & 0xe000) == 0x2000
                    && !(s[0] == 0x2001 && (s[1] == 0x0db8 || s[1] == 0))
                    && s[0] != 0x2002
            }
        }
    }
}
fn validate_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "URL 格式错误")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err("仅允许无凭据的公开 HTTPS（443）地址".into());
    }
    let host = url.host_str().ok_or("URL 缺少主机")?;
    if host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return Err("不允许访问本机或私有网络".into());
    }
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        if !is_public(ip) {
            return Err("不允许私网、回环或保留地址".into());
        }
    }
    Ok(url)
}
async fn public_get(raw: &str) -> Result<(String, String, Vec<u8>), String> {
    let mut url = validate_url(raw)?;
    for _ in 0..=4 {
        let host = url.host_str().ok_or("主机不存在")?.trim_matches(['[', ']']);
        let addresses =
            tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, 443)))
                .await
                .map_err(|_| "DNS 解析超时")?
                .map_err(|e| e.to_string())?
                .collect::<Vec<_>>();
        if addresses.is_empty() || addresses.iter().any(|a| !is_public(a.ip())) {
            return Err("DNS 返回非公开网络地址，已阻止访问".into());
        }
        // Pin validated addresses and disable proxy/automatic redirects to avoid DNS rebinding.
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
            .get(url.clone())
            .header("User-Agent", "DynamicIsland/0.1 (public-page-reader)")
            .send()
            .await
            .map_err(|_| "网页请求失败或超时")?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|h| h.to_str().ok())
                .ok_or("重定向缺少有效地址")?;
            url = validate_url(url.join(location).map_err(|_| "重定向格式错误")?.as_str())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("网页服务返回 HTTP {}", response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > BODY_LIMIT as u64)
        {
            return Err("网页响应超过 512 KiB 限制".into());
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let mut bytes = vec![];
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "读取网页响应中断")?;
            if bytes.len() + chunk.len() > BODY_LIMIT {
                return Err("网页响应超过 512 KiB 限制".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok((url.to_string(), content_type, bytes));
    }
    Err("网页重定向次数超过限制".into())
}

#[derive(Deserialize)]
struct Rss {
    channel: Channel,
}
#[derive(Deserialize)]
struct Channel {
    #[serde(default)]
    item: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    title: String,
    link: String,
    #[serde(default)]
    description: String,
}
fn strip_html(raw: &str) -> String {
    Html::parse_fragment(raw)
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
}
fn parse_results(provider: SearchProvider, bytes: &[u8]) -> Result<Vec<SearchResult>, String> {
    let mut results = match provider {
        SearchProvider::BingPublic => {
            let xml = std::str::from_utf8(bytes).map_err(|_| "搜索服务编码不是 UTF-8")?;
            let rss: Rss = quick_xml::de::from_str(xml)
                .map_err(|_| "Bing 未返回有效搜索结果，可能限流或接口不可用")?;
            rss.channel
                .item
                .into_iter()
                .map(|i| SearchResult {
                    title: i.title,
                    url: i.link,
                    published_at: None,
                    snippet: strip_html(&i.description),
                    source: "Bing".into(),
                })
                .collect::<Vec<_>>()
        }
        SearchProvider::Searxng => {
            let value: Value = serde_json::from_slice(bytes)
                .map_err(|_| "SearXNG 未返回 JSON，请确认实例启用了 JSON 输出")?;
            value["results"]
                .as_array()
                .ok_or("搜索结果缺少 results")?
                .iter()
                .filter_map(|v| {
                    Some(SearchResult {
                        title: v["title"].as_str()?.into(),
                        url: v["url"].as_str()?.into(),
                        published_at: v["publishedDate"].as_str().map(str::to_owned),
                        snippet: v["content"].as_str().unwrap_or("").into(),
                        source: "SearXNG".into(),
                    })
                })
                .collect()
        }
        SearchProvider::Unconfigured => return Err("尚未配置搜索后端".into()),
    };
    let mut urls = HashSet::new();
    results.retain(|r| validate_url(&r.url).is_ok() && urls.insert(r.url.clone()));
    results.truncate(8);
    for result in &mut results {
        result.title = result.title.chars().take(200).collect();
        result.snippet = result.snippet.chars().take(800).collect();
    }
    Ok(results)
}
pub async fn search(
    app: &tauri::AppHandle,
    args: SearchArgs,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> ToolExecutionResult {
    let config = match read_config(app) {
        Ok(c) => c,
        Err(_) => {
            return ToolExecutionResult::failure("SEARCH_CONFIG_ERROR", "搜索配置无法读取", false)
        }
    };
    if config.provider == SearchProvider::Unconfigured {
        return ToolExecutionResult::failure("SEARCH_NOT_CONFIGURED","尚未选择搜索后端。请在设置中心的网络与代理中配置 Bing 公开搜索或 SearXNG；不能假装已联网搜索。",false);
    }
    let mut url = match validate_url(if config.provider == SearchProvider::BingPublic {
        "https://www.bing.com/search"
    } else {
        &config.endpoint
    }) {
        Ok(u) => u,
        Err(e) => return ToolExecutionResult::failure("SEARCH_CONFIG_ERROR", e, false),
    };
    url.query_pairs_mut()
        .append_pair("q", &args.query)
        .append_pair(
            "format",
            if config.provider == SearchProvider::BingPublic {
                "rss"
            } else {
                "json"
            },
        );
    let operation = async {
        let (_, _, bytes) = public_get(url.as_str()).await?;
        parse_results(config.provider, &bytes)
    };
    match cancellable(operation, cancelled).await {
        Ok(results) => {
            let mut result = ToolExecutionResult::success(
                "已取得真实搜索结果",
                json!({"query":args.query,"sources":results,"untrusted":true}),
            );
            result.display = Some(json!({"kind":"sources","sources":results}));
            result
        }
        Err(error) => error,
    }
}
pub async fn fetch(args: FetchArgs, cancelled: &(dyn Fn() -> bool + Sync)) -> ToolExecutionResult {
    let operation = async {
        let (final_url, kind, bytes) = public_get(&args.url).await?;
        if !(kind.contains("text/html") || kind.contains("text/plain")) {
            return Err("不支持的网页类型，只允许 HTML 或纯文本".into());
        }
        let raw = String::from_utf8(bytes).map_err(|_| "此网页不是 UTF-8，暂不支持")?;
        let (title, text) = if kind.contains("text/html") {
            page_text(&raw)
        } else {
            (String::new(), raw)
        };
        let truncated = text.chars().count() > 12000;
        Ok(
            json!({"url":args.url,"finalUrl":final_url,"title":title,"text":text.chars().take(12000).collect::<String>(),"truncated":truncated,"untrusted":true}),
        )
    };
    match cancellable(operation, cancelled).await {
        Ok(data) => ToolExecutionResult::success("已读取公开网页", data),
        Err(e) => e,
    }
}
fn page_text(raw: &str) -> (String, String) {
    let html = Html::parse_document(raw);
    let title = html
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default();
    let root = html
        .select(&Selector::parse("article, main").unwrap())
        .next()
        .or_else(|| html.select(&Selector::parse("body").unwrap()).next())
        .unwrap_or_else(|| html.root_element());
    let mut parts = vec![];
    for node in root.descendants() {
        if node.ancestors().any(|a| {
            a.value().as_element().is_some_and(|e| {
                matches!(
                    e.name(),
                    "script" | "style" | "noscript" | "svg" | "template"
                )
            })
        }) {
            continue;
        }
        if let Some(text) = node.value().as_text() {
            let text = text.trim();
            if !text.is_empty() {
                parts.push(text);
            }
        }
    }
    (title.chars().take(200).collect(), parts.join("\n"))
}
async fn cancellable<T>(
    future: impl std::future::Future<Output = Result<T, String>>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<T, ToolExecutionResult> {
    if cancelled() {
        return Err(native_dialog::cancelled_result());
    }
    tokio::pin!(future);
    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result=&mut future=> { if cancelled() { return Err(native_dialog::cancelled_result()); } return result.map_err(|e|ToolExecutionResult::failure("WEB_REQUEST_FAILED",e,false)); },
            _=&mut deadline=>return Err(ToolExecutionResult::failure("WEB_TIMEOUT","网络工具超过 30 秒限制",true)),
            _=poll.tick()=>if cancelled() { return Err(native_dialog::cancelled_result()); },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_private_urls_and_mapped_ips() {
        for url in [
            "http://example.org",
            "https://localhost",
            "https://127.0.0.1",
            "https://169.254.169.254",
            "https://[::1]",
            "https://[::ffff:127.0.0.1]",
            "https://user:secret@example.org",
            "https://example.org:8443",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
        assert!(validate_url("https://example.org").is_ok());
        assert!(!is_public("100.64.1.1".parse().unwrap()));
    }
    #[test]
    fn real_provider_results_keep_sources_and_filter_duplicates() {
        let bytes=br#"<rss><channel><item><title>Rust</title><link>https://www.rust-lang.org/</link><description>Official site</description></item><item><title>duplicate</title><link>https://www.rust-lang.org/</link><description>x</description></item><item><title>private</title><link>https://127.0.0.1</link><description>x</description></item></channel></rss>"#;
        let result = parse_results(SearchProvider::BingPublic, bytes).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source, "Bing");
        assert!(parse_results(SearchProvider::BingPublic, b"<html>captcha</html>").is_err());
        let (title,text)=page_text("<title>Example</title><body><main><h1>Hello</h1><script>secret()</script><p>World</p></main></body>");
        assert_eq!(title, "Example");
        assert!(text.contains("World"));
        assert!(!text.contains("secret"));
    }
}
