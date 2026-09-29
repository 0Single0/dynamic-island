//! Markdown HTML → PDF：使用 Windows WebView2 的原生打印接口。

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::{WebviewWindowBuilder, WebviewUrl};

#[tauri::command]
pub fn read_markdown_file(path: String) -> Result<String, String> {
    let is_markdown = std::path::Path::new(&path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("md") || value.eq_ignore_ascii_case("markdown"))
        .unwrap_or(false);
    if !is_markdown {
        return Err("只支持 .md 或 .markdown 文件".into());
    }
    std::fs::read_to_string(&path)
        .map(|text| text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
        .map_err(|error| format!("读取 Markdown 文件失败: {error}"))
}

#[tauri::command]
pub fn read_markdown_image_data(
    image_path: String,
    markdown_path: String,
) -> Result<String, String> {
    let raw_path = image_path.trim();
    if raw_path.is_empty() {
        return Err("图片路径为空".into());
    }

    let normalized_path = raw_path
        .strip_prefix("file:///")
        .unwrap_or(raw_path)
        .replace('/', "\\");
    let image = std::path::Path::new(&normalized_path);
    let resolved = if image.is_absolute() {
        image.to_path_buf()
    } else {
        std::path::Path::new(&markdown_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(image)
    };

    let bytes = std::fs::read(&resolved)
        .map_err(|error| format!("读取图片失败 [{}]: {error}", resolved.display()))?;
    let mime = match resolved
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    };

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

#[tauri::command]
pub async fn markdown_to_pdf(
    app: tauri::AppHandle,
    html: String,
    output_path: String,
    title: String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        render_with_webview2(&app, &html, &output_path, &title).await
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, html, output_path, title);
        Err("Markdown 转 PDF 当前仅支持 Windows".into())
    }
}

#[cfg(target_os = "windows")]
async fn render_with_webview2(
    app: &tauri::AppHandle,
    html: &str,
    output_path: &str,
    title: &str,
) -> Result<(), String> {
    let label = format!("markdown-print-{}", uuid_suffix());
    let print_window = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App("index.html".into()),
    )
    .title(title)
    .inner_size(794.0, 1123.0)
    .position(-2000.0, -2000.0)
    .decorations(false)
    .resizable(false)
    .skip_taskbar(true)
    .visible(false)
    .build()
    .map_err(|e| format!("创建打印页面失败: {e}"))?;

    let document = printable_document(html, title);
    if let Err(error) = navigate_to_string(&print_window, &document) {
        let _ = print_window.destroy();
        return Err(error);
    }
    tokio::time::sleep(Duration::from_millis(650)).await;

    let result = print_to_pdf(&print_window, output_path).await;
    let _ = print_window.destroy();
    result
}

#[cfg(target_os = "windows")]
fn navigate_to_string(window: &tauri::WebviewWindow, html: &str) -> Result<(), String> {
    use std::sync::mpsc;

    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
    use windows::core::PCWSTR;

    let content: Vec<u16> = html.encode_utf16().chain(std::iter::once(0)).collect();
    let (sender, receiver) = mpsc::channel::<Result<(), String>>();
    window
        .with_webview(move |webview| unsafe {
            let result = webview
                .controller()
                .CoreWebView2()
                .and_then(|core: ICoreWebView2| {
                    core.NavigateToString(PCWSTR(content.as_ptr()))
                })
                .map_err(|e| e.to_string());
            let _ = sender.send(result);
        })
        .map_err(|e| format!("加载打印页面失败: {e}"))?;
    receiver
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "加载打印页面超时".to_string())?
}

#[cfg(target_os = "windows")]
async fn print_to_pdf(window: &tauri::WebviewWindow, output_path: &str) -> Result<(), String> {
    use std::sync::mpsc;

    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{ICoreWebView2_7, ICoreWebView2Environment6},
        PrintToPdfCompletedHandler,
    };
    use windows::core::PCWSTR;
    use windows_core::Interface;

    let output: Vec<u16> = output_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let (sender, receiver) = mpsc::channel::<Result<bool, String>>();
    let callback_sender = sender.clone();

    window
        .with_webview(move |webview| unsafe {
            let result: Result<(), String> = (|| {
                let core: ICoreWebView2_7 = webview
                    .controller()
                    .CoreWebView2()
                    .map_err(|e| e.to_string())?
                    .cast()
                    .map_err(|e| e.to_string())?;
                let environment: ICoreWebView2Environment6 = webview
                    .environment()
                    .cast()
                    .map_err(|e| e.to_string())?;
                let settings = environment.CreatePrintSettings().map_err(|e| e.to_string())?;
                // WebView2 使用英寸；与打印 CSS 的 A4 纵向尺寸保持一致。
                settings.SetPageWidth(8.27).map_err(|e| e.to_string())?;
                settings.SetPageHeight(11.69).map_err(|e| e.to_string())?;
                // 页面边距由 HTML 的 @page 规则统一控制，避免重复叠加。
                settings.SetMarginTop(0.0).map_err(|e| e.to_string())?;
                settings.SetMarginBottom(0.0).map_err(|e| e.to_string())?;
                settings.SetMarginLeft(0.0).map_err(|e| e.to_string())?;
                settings.SetMarginRight(0.0).map_err(|e| e.to_string())?;
                settings.SetShouldPrintBackgrounds(true).map_err(|e| e.to_string())?;
                settings.SetShouldPrintHeaderAndFooter(false).map_err(|e| e.to_string())?;

                let handler = PrintToPdfCompletedHandler::create(Box::new(move |error, success| {
                    let result = error.map(|_| success).map_err(|e| e.to_string());
                    let _ = callback_sender.send(result);
                    Ok(())
                }));

                core.PrintToPdf(PCWSTR(output.as_ptr()), &settings, &handler)
                    .map_err(|e| e.to_string())
            })();
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        })
        .map_err(|e| format!("启动 PDF 打印失败: {e}"))?;

    let completed = tokio::task::spawn_blocking(move || {
        receiver
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| "PDF 打印超时".to_string())?
    })
    .await
    .map_err(|e| format!("等待 PDF 打印失败: {e}"))??;

    if completed {
        Ok(())
    } else {
        Err("WebView2 未能完成 PDF 打印".into())
    }
}

#[cfg(target_os = "windows")]
fn printable_document(body: &str, title: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>{}</title>
<style>
@page {{ size: A4 portrait; margin: 18mm 16mm 20mm; }}
* {{ box-sizing: border-box; }}
html, body {{ margin: 0; padding: 0; background: #fff; }}
body {{ color: #202124; font-family: "Segoe UI", "Microsoft YaHei", Arial, sans-serif; font-size: 11pt; line-height: 1.65; }}
main {{ max-width: 100%; overflow-wrap: anywhere; }}
h1, h2, h3, h4 {{ color: #111827; line-height: 1.25; margin: 0.8em 0 0.35em; break-after: avoid-page; page-break-after: avoid; }}
h1 {{ font-size: 23pt; }} h2 {{ font-size: 17pt; }} h3 {{ font-size: 13pt; }}
p {{ margin: 0.5em 0; }} ul, ol {{ margin: 0.45em 0; padding-left: 1.5em; }}
 blockquote {{ margin: 0.7em 0; border-left: 3px solid #9ca3af; padding-left: 0.8em; color: #4b5563; break-inside: avoid; }}
pre {{ margin: 0.7em 0; padding: 0.8em; overflow-wrap: anywhere; white-space: pre-wrap; background: #f3f4f6; border: 1px solid #e5e7eb; border-radius: 6px; break-inside: avoid; }}
code {{ font-family: Consolas, "Cascadia Code", monospace; font-size: 0.9em; }} pre code {{ white-space: pre-wrap; }}
a {{ color: #2563eb; }} img {{ max-width: 100%; height: auto; break-inside: avoid; page-break-inside: avoid; }}
table {{ width: 100%; border-collapse: collapse; margin: 0.75em 0; break-inside: auto; }} thead {{ display: table-header-group; }}
tr {{ break-inside: avoid; page-break-inside: avoid; }} th, td {{ border: 1px solid #d1d5db; padding: 0.35em 0.5em; text-align: left; vertical-align: top; }}
th {{ background: #f3f4f6; color: #111827; }}
</style></head><body><main>{}</main></body></html>"#,
        escape_html(title), body
    )
}

#[cfg(target_os = "windows")]
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(target_os = "windows")]
fn uuid_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|_| "temporary".into())
}
