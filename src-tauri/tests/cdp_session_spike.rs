use std::{env, time::Duration};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type CdpSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn send_command(
    socket: &mut CdpSocket,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    socket
        .send(Message::Text(
            json!({"id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|error| error.to_string())?;
    while let Some(message) = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .map_err(|_| format!("CDP command timed out: {method}"))?
    {
        let message = message.map_err(|error| error.to_string())?;
        if let Message::Text(text) = message {
            let response: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            if response["id"] == id {
                if !response["error"].is_null() {
                    return Err(response["error"].to_string());
                }
                return Ok(response["result"].clone());
            }
        }
    }
    Err(format!("CDP socket closed before response: {method}"))
}

#[tokio::test]
async fn isolated_profile_discovers_creates_uses_and_disposes_target() {
    let Ok(port) = env::var("DYNAMIC_ISLAND_CDP_SPIKE_PORT") else {
        eprintln!("set DYNAMIC_ISLAND_CDP_SPIKE_PORT to run the local isolated-browser spike");
        return;
    };
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let version: Value = client
        .get(format!("{base}/json/version"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(version["Browser"]
        .as_str()
        .unwrap_or("")
        .starts_with("Chrome/"));
    let browser_ws_url = version["webSocketDebuggerUrl"].as_str().unwrap();
    let parsed_ws = reqwest::Url::parse(browser_ws_url).unwrap();
    assert_eq!(parsed_ws.scheme(), "ws");
    assert_eq!(parsed_ws.host_str(), Some("127.0.0.1"));
    assert_eq!(parsed_ws.port(), port.parse::<u16>().ok());

    let (mut browser, _) = connect_async(browser_ws_url).await.unwrap();
    let mut id = 1;
    let context = send_command(
        &mut browser,
        id,
        "Target.createBrowserContext",
        json!({"disposeOnDetach":true}),
    )
    .await
    .unwrap();
    id += 1;
    let context_id = context["browserContextId"].as_str().unwrap().to_owned();
    let target_result = send_command(
        &mut browser,
        id,
        "Target.createTarget",
        json!({"url":"about:blank","browserContextId":context_id,"background":true}),
    )
    .await
    .unwrap();
    let target_id = target_result["targetId"].as_str().unwrap().to_owned();

    let test_result = async {
        let targets: Vec<Value> = client.get(format!("{base}/json/list")).send().await.map_err(|error| error.to_string())?.json().await.map_err(|error| error.to_string())?;
        let target = targets.iter().find(|target| target["id"] == target_id && target["type"] == "page")
            .ok_or_else(|| "created Target was not discoverable as a page".to_owned())?;
        let page_ws = target["webSocketDebuggerUrl"].as_str().ok_or_else(|| "page Target omitted its WebSocket URL".to_owned())?;
        let (mut page, _) = connect_async(page_ws).await.map_err(|error| error.to_string())?;
        send_command(&mut page, 1, "Page.enable", json!({})).await?;
        let evaluated = send_command(&mut page, 2, "Runtime.evaluate", json!({"expression":"({href: location.href, title: document.title})","returnByValue":true})).await?;
        assert_eq!(evaluated["result"]["value"]["href"], "about:blank");
        let _ = page.close(None).await;
        Ok::<(), String>(())
    }.await;

    let _ = send_command(
        &mut browser,
        id + 1,
        "Target.closeTarget",
        json!({"targetId":target_id}),
    )
    .await;
    let _ = send_command(
        &mut browser,
        id + 2,
        "Target.disposeBrowserContext",
        json!({"browserContextId":context_id}),
    )
    .await;
    let _ = send_command(&mut browser, id + 3, "Browser.close", json!({})).await;
    let _ = browser.close(None).await;
    test_result.unwrap();

    let remaining: Vec<Value> = client
        .get(format!("{base}/json/list"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!remaining.iter().any(|target| target["id"] == target_id));
}
