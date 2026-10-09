//! Opt-in live diagnostics. Credentials are loaded directly from the OS vault.
#[cfg(test)]
mod tests {
    use super::super::{
        provider_error::ProviderError,
        secrets::{OsCredentialStore, SecretStore},
    };
    use rusqlite::{Connection, OpenFlags};
    use serde_json::json;

    struct FixtureRead;
    #[async_trait::async_trait]
    impl super::super::tools::ModelToolExecutor for FixtureRead {
        async fn execute(
            &self,
            name: &str,
            args: serde_json::Value,
        ) -> super::super::tools::ToolExecutionResult {
            assert_eq!(name, "netease.get_current_state");
            assert!(args.is_object());
            super::super::tools::ToolExecutionResult::success(
                "Validation fixture, not the user's music",
                json!({"connection":"idle","track":null,"isPlaying":false,"positionMs":0}),
            )
        }
    }

    struct GeneralFixture(std::sync::Mutex<Vec<String>>);
    #[async_trait::async_trait]
    impl super::super::tools::ModelToolExecutor for GeneralFixture {
        async fn execute(
            &self,
            name: &str,
            args: serde_json::Value,
        ) -> super::super::tools::ToolExecutionResult {
            self.0.lock().unwrap().push(name.to_owned());
            if name == "netease.start_or_connect" {
                assert_eq!(args, json!({}));
                super::super::tools::ToolExecutionResult::failure(
                    "USER_DENIED",
                    "测试执行器模拟用户拒绝启动。没有启动任何真实应用，不要重试。",
                    false,
                )
            } else {
                super::super::tools::ToolExecutionResult::failure(
                    "TEST_UNEXPECTED_TOOL",
                    "测试未请求读取本机信息",
                    false,
                )
            }
        }
    }

    #[tokio::test]
    #[ignore = "live greeting/launch selection regression; tool execution is mocked, never launches an app"]
    async fn live_ark_general_agent_behavior() {
        use super::super::{
            adapter::{ModelRequest, ProviderAdapter},
            model::{ModelMessage, SYSTEM_PROMPT},
            native_trace::{run_loop, DurableTrace},
            plugins::PluginRuntime,
            providers::ProviderProtocol,
            storage::AgentStore,
        };
        let runtime = std::sync::Arc::new(PluginRuntime::new());
        super::super::plugins::mount_builtin_plugins(&runtime, None)
            .await
            .unwrap();
        let tools = runtime.all_tools().await;
        let db = std::env::var("APPDATA").unwrap() + "/com.issuser.dynamic-island/agent.sqlite3";
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut statement = conn.prepare("SELECT p.base_url,p.secret_ref,m.model_id FROM providers p JOIN models m ON m.provider_id=p.id WHERE p.enabled=1 AND m.enabled=1 AND p.base_url LIKE 'https://ark.cn-beijing.volces.com/%'").unwrap();
        let configs = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!configs.is_empty(), "no enabled Ark configuration");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap();
        for (base, reference, model) in configs {
            let key = OsCredentialStore.get(&reference).unwrap().unwrap();
            let adapter =
                ProviderAdapter::configured(ProviderProtocol::ChatCompletions, &base, &model);
            for (scenario, content) in [("greeting", "你好"), ("launch", "你帮我启动下网易云")]
            {
                let path = std::env::temp_dir().join(format!(
                    "agent-general-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                let store = AgentStore::open(&path).unwrap();
                let session = store.create_session("general regression fixture").unwrap();
                store.create_run("fixture", &session.id).unwrap();
                let trace = DurableTrace {
                    store: &store,
                    app: None,
                    session_id: &session.id,
                    run_id: "fixture",
                };
                let executor = GeneralFixture(std::sync::Mutex::new(vec![]));
                let request = ModelRequest {
                    model: model.clone(),
                    system: SYSTEM_PROMPT.into(),
                    messages: vec![ModelMessage::text("user", content)],
                    tools: tools.clone(),
                    max_tokens: 2048,
                    stream: true,
                };
                let result = run_loop(
                    &trace,
                    &executor,
                    &super::super::native_trace::ToolProvenance::new(),
                    || false,
                    &client,
                    &adapter,
                    &adapter.endpoint(&base),
                    key.as_str(),
                    request,
                )
                .await;
                let events = store.list_events(&session.id, 0).unwrap();
                let calls = executor.0.lock().unwrap().clone();
                drop(store);
                std::fs::remove_dir_all(path).unwrap();
                let answer = result.unwrap();
                println!("GENERAL_REGRESSION model={model} scenario={scenario} calls={calls:?} answer={answer}");
                assert!(!answer.trim().is_empty());
                if scenario == "greeting" {
                    assert!(calls.is_empty(), "greeting must not invoke any tool");
                    assert!(
                        !answer.contains("网易云"),
                        "greeting must not advertise the music plugin"
                    );
                } else {
                    assert_eq!(calls, vec!["netease.start_or_connect"], "launch request must not be replaced by a music read or retried after denial");
                    assert!(events.iter().any(|e| e.event_type == "tool_completed"
                        && e.payload["errorCode"] == "USER_DENIED"));
                }
            }
        }
    }

    #[tokio::test]
    #[ignore = "authenticates to configured Ark models and runs the production tool loop with a fixture"]
    async fn live_ark_production_tool_loop() {
        use super::super::{
            adapter::{ModelRequest, ProviderAdapter},
            model::{ModelMessage, SYSTEM_PROMPT},
            native_trace::{run_loop, DurableTrace},
            plugins::PluginRuntime,
            providers::ProviderProtocol,
            storage::AgentStore,
        };
        let runtime = std::sync::Arc::new(PluginRuntime::new());
        super::super::plugins::mount_builtin_plugins(&runtime, None)
            .await
            .unwrap();
        let tools = runtime.all_tools().await;
        let db_path =
            std::env::var("APPDATA").unwrap() + "/com.issuser.dynamic-island/agent.sqlite3";
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut stmt=conn.prepare("SELECT p.base_url,p.secret_ref,m.model_id FROM providers p JOIN models m ON m.provider_id=p.id WHERE p.enabled=1 AND m.enabled=1 AND p.base_url LIKE 'https://ark.cn-beijing.volces.com/%'").unwrap();
        let configs = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!configs.is_empty());
        for (base, reference, model) in configs {
            let key = OsCredentialStore.get(&reference).unwrap().unwrap();
            let path = std::env::temp_dir().join(format!(
                "agent-live-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let store = AgentStore::open(&path).unwrap();
            let session = store.create_session("live test").unwrap();
            store.create_run("live-run", &session.id).unwrap();
            store.seed_context(&session.id).unwrap();
            store.append_context(&session.id,"live-run", &[ModelMessage::text("user","请调用网易云读取工具查询当前播放状态。必须先调用工具，不能猜测；只需简短回答。")]).unwrap();
            let trace = DurableTrace {
                store: &store,
                app: None,
                session_id: &session.id,
                run_id: "live-run",
            };
            let request=ModelRequest{model:model.clone(),system:SYSTEM_PROMPT.into(),messages:vec![ModelMessage::text("user","请调用网易云读取工具查询当前播放状态。必须先调用工具，不能猜测；只需简短回答。")],tools:tools.clone(),max_tokens:8192,stream:true};
            let adapter =
                ProviderAdapter::configured(ProviderProtocol::ChatCompletions, &base, &model);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap();
            let result = run_loop(
                &trace,
                &FixtureRead,
                &super::super::native_trace::ToolProvenance::new(),
                || false,
                &client,
                &adapter,
                &adapter.endpoint(&base),
                key.as_str(),
                request,
            )
            .await;
            match result {
                Ok(answer) => {
                    let events = store.list_events(&session.id, 0).unwrap();
                    let calls = events
                        .iter()
                        .filter(|e| e.event_type == "tool_completed")
                        .count();
                    let deltas = events
                        .iter()
                        .filter(|e| e.event_type == "text_delta")
                        .count();
                    let reasoning = events
                        .iter()
                        .filter(|e| e.event_type == "reasoning_delta")
                        .count();
                    println!("LIVE_LOOP model={model} tool_results={calls} text_deltas={deltas} reasoning_deltas={reasoning} answer_chars={}",answer.chars().count());
                    assert!(calls > 0);
                    assert!(deltas > 0);
                    drop(store);
                    let store = AgentStore::open(&path).unwrap();
                    let mut messages = store.list_context(&session.id).unwrap();
                    messages.push(ModelMessage::text(
                        "user",
                        "刚才工具返回的是不是没有播放？简短回答。",
                    ));
                    let continuation = ModelRequest {
                        model: model.clone(),
                        system: SYSTEM_PROMPT.into(),
                        messages,
                        tools: tools.clone(),
                        max_tokens: 2048,
                        stream: true,
                    };
                    let response = adapter
                        .stream_step(
                            &client,
                            &adapter.endpoint(&base),
                            key.as_str(),
                            &continuation,
                            || false,
                            |_| Ok(()),
                        )
                        .await
                        .unwrap();
                    assert!(!response.plain_text().is_empty());
                    println!("LIVE_LOOP model={model} restored_context_status=200");
                }
                Err(error) => panic!("{}", serde_json::to_string(&error).unwrap()),
            }
            std::fs::remove_dir_all(path).unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "uses configured Ark models and small amounts of live API quota"]
    async fn live_ark_request_matrix() {
        let path = std::env::var("APPDATA").unwrap() + "/com.issuser.dynamic-island/agent.sqlite3";
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut statement = conn.prepare("SELECT p.base_url, p.secret_ref, m.model_id FROM providers p JOIN models m ON m.provider_id=p.id WHERE p.enabled=1 AND m.enabled=1 AND p.base_url LIKE 'https://ark.cn-beijing.volces.com/%'").unwrap();
        let configs = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!configs.is_empty(), "no enabled Ark configuration");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap();
        for (base, reference, model) in configs {
            let key = OsCredentialStore
                .get(&reference)
                .unwrap()
                .expect("missing credential");
            let endpoint = if base.ends_with("/chat/completions") {
                base
            } else {
                format!("{}/chat/completions", base.trim_end_matches('/'))
            };
            for mode in [
                "minimal",
                "original",
                "valid_tools",
                "max_tokens",
                "max_completion_tokens",
                "usage",
            ] {
                let mut body = json!({"model":model,"messages":[{"role":"user","content":"Reply with OK only."}],"stream":true});
                if matches!(mode, "original" | "valid_tools") {
                    body["tools"] = json!([{"type":"function","function":{"name":if mode=="original" {"netease.get_current_state"} else {"netease_get_current_state"},"description":"Read music status","parameters":{"type":"object","properties":{}}}}]);
                    body["tool_choice"] = json!("auto");
                }
                if mode == "original" || mode == "max_completion_tokens" {
                    body["max_completion_tokens"] = json!(2048);
                }
                if mode == "max_tokens" {
                    body["max_tokens"] = json!(2048);
                }
                if mode == "original" || mode == "usage" {
                    body["stream_options"] = json!({"include_usage":true});
                }
                let response = client
                    .post(&endpoint)
                    .bearer_auth(key.as_str())
                    .json(&body)
                    .send()
                    .await
                    .unwrap();
                let status = response.status().as_u16();
                if response.status().is_success() {
                    let text = response.text().await.unwrap();
                    println!(
                        "PROBE model={model} mode={mode} status={status} completed={}",
                        text.contains("[DONE]")
                    );
                } else {
                    let error = ProviderError::from_response(
                        response,
                        "chat-completions",
                        &model,
                        &endpoint,
                        key.as_str(),
                    )
                    .await;
                    println!(
                        "PROBE model={model} mode={mode} error={}",
                        serde_json::to_string(&error).unwrap()
                    );
                }
            }
        }
    }
}
