//! 模型 × 渠道能力矩阵：路由的第一层过滤器。
//!
//! 「glm 只能走 workbuddy、ds 能走 workbuddy + qoder」这件事**不写死在代码里**：
//! 各渠道目录里有什么模型，矩阵里就有什么候选；渠道上新一次目录刷新即生效。
//! 人工只做覆盖（别名、优先级、上限、临时禁用）。

use std::collections::HashMap;

use crate::gateway::{
    channels::{ModelCapabilities, UpstreamModel},
    config::RouteOverride,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSource {
    /// 渠道实时目录。
    #[default]
    Live,
    /// 内置快照（实时拉取失败时的回落）。
    Builtin,
    /// 人工覆盖。
    Override,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ModelLimits {
    pub context: Option<u64>,
    pub max_output: Option<u64>,
}

/// 一条「对外模型 → 某渠道」的路由。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ModelRoute {
    pub public_model: String,
    pub channel_id: String,
    /// 上游真实模型名（可能与对外名不同）。
    pub upstream_model: String,
    pub priority: u32,
    pub weight: u32,
    pub enabled: bool,
    pub credits: Option<String>,
    pub limits: ModelLimits,
    pub capabilities: ModelCapabilities,
    /// 上下文档位 / 思考档位 / 采样参数（面板要能一眼看清这个模型怎么用）。
    pub context_windows: Vec<u64>,
    pub reasoning_efforts: Vec<String>,
    pub reasoning_fixed_effort: Option<String>,
    pub reasoning_default_effort: Option<String>,
    pub reasoning_can_disable: Option<bool>,
    pub always_reasoning: bool,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    /// 能力字段是否来自实时元数据。快照/缺字段时为 false：
    /// 此时不做能力过滤（乐观放行，让上游报错），避免"因为未知就一律拒绝"。
    pub caps_known: bool,
    pub source: RouteSource,
}

impl ModelRoute {
    pub fn is_free(&self) -> bool {
        crate::gateway::channels::credits_is_free(self.credits.as_deref())
    }
}

/// 请求对候选渠道的硬性要求。不满足的候选在本地就被剔除（不发上游、不耗额度）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Requirements {
    pub vision: bool,
    pub tools: bool,
    /// 输入 token 估算值。
    pub context: u64,
    pub max_output: Option<u64>,
}

impl Requirements {
    pub fn from_request(
        request: &crate::gateway::protocol::cir::CanonicalRequest,
        estimated_prompt_tokens: u64,
    ) -> Self {
        let vision = request.messages.iter().any(|message| {
            message
                .content
                .iter()
                .any(|block| matches!(block, crate::gateway::protocol::cir::Block::Image { .. }))
        });
        Self {
            vision,
            tools: !request.tools.is_empty(),
            context: estimated_prompt_tokens,
            max_output: request.max_tokens,
        }
    }
}

/// 一条渠道的目录快照。
pub struct ChannelCatalog {
    pub channel_id: String,
    pub priority: u32,
    pub weight: u32,
    pub models: Vec<UpstreamModel>,
    /// 目录是否来自实时拉取（决定 source 标记）。
    pub live: bool,
}

#[derive(Debug, Default)]
pub struct Matrix {
    routes: HashMap<String, Vec<ModelRoute>>,
    aliases: HashMap<String, String>,
    built_at_ms: u64,
}

impl Matrix {
    pub fn build(
        catalogs: &[ChannelCatalog],
        aliases: &HashMap<String, String>,
        overrides: &[RouteOverride],
    ) -> Self {
        let mut routes: HashMap<String, Vec<ModelRoute>> = HashMap::new();

        for catalog in catalogs {
            for model in &catalog.models {
                // 即使实时目录里出现了内部模型，快照回落时也不该把它当聊天模型上架。
                if !is_public_model(&model.id) {
                    continue;
                }
                let source = if catalog.live && !model.from_snapshot {
                    RouteSource::Live
                } else {
                    RouteSource::Builtin
                };
                routes
                    .entry(model.id.clone())
                    .or_default()
                    .push(ModelRoute {
                        public_model: model.id.clone(),
                        channel_id: catalog.channel_id.clone(),
                        upstream_model: model.id.clone(),
                        priority: catalog.priority,
                        weight: catalog.weight,
                        enabled: true,
                        credits: model.credits.clone(),
                        limits: ModelLimits {
                            context: model.context_length,
                            max_output: model.max_output_tokens,
                        },
                        capabilities: model.capabilities.clone(),
                        context_windows: model.context_windows.clone(),
                        reasoning_efforts: model.reasoning_efforts.clone(),
                        reasoning_fixed_effort: model.reasoning_fixed_effort.clone(),
                        reasoning_default_effort: model.reasoning_default_effort.clone(),
                        reasoning_can_disable: model.reasoning_can_disable,
                        always_reasoning: model.always_reasoning,
                        temperature: model.temperature,
                        top_p: model.top_p,
                        // 快照条目没有可信的能力元数据，标记为未知以免误杀请求。
                        caps_known: !model.from_snapshot && has_capability_meta(model),
                        source,
                    });
            }
        }

        apply_overrides(&mut routes, overrides);

        Self {
            routes,
            aliases: aliases.clone(),
            built_at_ms: crate::gateway::timeutil::now_ms(),
        }
    }

    pub fn built_at_ms(&self) -> u64 {
        self.built_at_ms
    }

    /// 解析别名后取出该对外模型的候选渠道（已剔除禁用的）。
    pub fn routes(&self, public_model: &str) -> Vec<ModelRoute> {
        let canonical = self.canonical(public_model);
        self.routes
            .get(&canonical)
            .map(|routes| {
                routes
                    .iter()
                    .filter(|route| route.enabled)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn canonical<'a>(&'a self, public_model: &'a str) -> String {
        self.aliases
            .get(public_model)
            .cloned()
            .unwrap_or_else(|| public_model.to_string())
    }

    /// 对外模型清单（含别名），供 `/v1/models` 与面板使用。
    pub fn public_models(&self) -> Vec<String> {
        let mut models: Vec<String> = self.routes.keys().cloned().collect();
        models.sort();
        models
    }

    pub fn route_count(&self) -> usize {
        self.routes.values().map(Vec::len).sum()
    }

    pub fn sources(&self) -> (usize, usize) {
        let live = self
            .routes
            .values()
            .flatten()
            .filter(|route| route.source == RouteSource::Live)
            .count();
        let builtin = self
            .routes
            .values()
            .flatten()
            .filter(|route| route.source == RouteSource::Builtin)
            .count();
        (live, builtin)
    }

    /// 该候选是否满足请求的硬性要求；不满足时返回可读原因。
    pub fn reject_reason(route: &ModelRoute, requirements: &Requirements) -> Option<String> {
        if requirements.vision && route.caps_known && !route.capabilities.vision {
            return Some("该渠道不支持图片输入".to_string());
        }
        if requirements.tools && route.caps_known && !route.capabilities.tool_calls {
            return Some("该渠道不支持工具调用".to_string());
        }
        if let Some(context) = route.limits.context {
            if context > 0 && requirements.context > context {
                return Some(format!("上下文超出该渠道上限（{context}）"));
            }
        }
        // 注意：**不**用 max_tokens 过滤候选。
        // 客户端常发一个远大于上限的值（整段上下文长度、1M 之类），
        // 上游对这种情况是静默钳制到真实上限，而不是报错；
        // 网关照做即可（见 dispatch 里的钳制），拒绝反而会把唯一候选剔掉。
        None
    }
}

/// 内部/虚拟模型不上架（与渠道目录的清洗规则一致）。
fn is_public_model(id: &str) -> bool {
    crate::gateway::channels::is_chat_model(id)
}

/// 实时元数据里是否带了能力字段：没带就不能当成"不支持"。
fn has_capability_meta(model: &UpstreamModel) -> bool {
    model.capabilities.vision
        || model.capabilities.tool_calls
        || model.capabilities.reasoning
        || model.context_length.is_some()
}

fn apply_overrides(routes: &mut HashMap<String, Vec<ModelRoute>>, overrides: &[RouteOverride]) {
    for entry in overrides {
        let canonical = entry.model.clone();
        let existing = routes
            .get_mut(&canonical)
            .and_then(|list| list.iter_mut().find(|route| route.channel_id == entry.channel_id));

        match existing {
            Some(route) => {
                if let Some(upstream) = &entry.upstream_model {
                    route.upstream_model = upstream.clone();
                }
                if let Some(priority) = entry.priority {
                    route.priority = priority;
                }
                if let Some(weight) = entry.weight {
                    route.weight = weight;
                }
                if let Some(enabled) = entry.enabled {
                    route.enabled = enabled;
                }
                route.source = RouteSource::Override;
            }
            None => {
                // 目录里没有这条（渠道不提供该模型）：只有显式给了 upstream_model 才补一条，
                // 否则视为"配置写错"而不是"悄悄多一个候选"。
                let Some(upstream) = entry.upstream_model.clone() else {
                    continue;
                };
                routes.entry(canonical.clone()).or_default().push(ModelRoute {
                    public_model: canonical.clone(),
                    channel_id: entry.channel_id.clone(),
                    upstream_model: upstream,
                    priority: entry.priority.unwrap_or(100),
                    weight: entry.weight.unwrap_or(1),
                    enabled: entry.enabled.unwrap_or(true),
                    credits: None,
                    limits: ModelLimits::default(),
                    capabilities: ModelCapabilities::default(),
                    context_windows: Vec::new(),
                    reasoning_efforts: Vec::new(),
                    reasoning_fixed_effort: None,
                    reasoning_default_effort: None,
                    reasoning_can_disable: None,
                    always_reasoning: false,
                    temperature: None,
                    top_p: None,
                    caps_known: false,
                    source: RouteSource::Override,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(id: &str, credits: Option<&str>, context: Option<u64>) -> UpstreamModel {
        UpstreamModel {
            id: id.into(),
            credits: credits.map(str::to_string),
            context_length: context,
            capabilities: ModelCapabilities {
                vision: false,
                tool_calls: true,
                reasoning: true,
            },
            ..UpstreamModel::default()
        }
    }

    fn catalogs() -> Vec<ChannelCatalog> {
        vec![
            ChannelCatalog {
                channel_id: "wb-cn".into(),
                priority: 100,
                weight: 5,
                live: true,
                models: vec![
                    model("deepseek-v4.1-flash", Some("x0.00"), Some(1_000_000)),
                    model("glm-5.3", Some("x0.00"), None),
                ],
            },
            ChannelCatalog {
                channel_id: "qoder".into(),
                priority: 90,
                weight: 3,
                live: true,
                models: vec![model("deepseek-v4.1-flash", Some("x0.20"), Some(200_000))],
            },
        ]
    }

    #[test]
    fn a_model_present_in_two_channels_yields_two_candidates() {
        let matrix = Matrix::build(&catalogs(), &HashMap::new(), &[]);
        let routes = matrix.routes("deepseek-v4.1-flash");
        assert_eq!(routes.len(), 2, "ds 应该同时命中 wk 与 qoder");
        let channels: Vec<&str> = routes.iter().map(|route| route.channel_id.as_str()).collect();
        assert!(channels.contains(&"wb-cn") && channels.contains(&"qoder"));
    }

    #[test]
    fn a_model_present_in_one_channel_stays_single_candidate() {
        let matrix = Matrix::build(&catalogs(), &HashMap::new(), &[]);
        let routes = matrix.routes("glm-5.3");
        assert_eq!(routes.len(), 1, "glm 只有 wk 一个候选");
        assert_eq!(routes[0].channel_id, "wb-cn");
        // 不存在的模型没有候选（上层据此本地 400，而不是发上游）。
        assert!(matrix.routes("gpt-9").is_empty());
    }

    #[test]
    fn aliases_resolve_to_the_canonical_model() {
        let matrix = Matrix::build(
            &catalogs(),
            &[("ds-flash".to_string(), "deepseek-v4.1-flash".to_string())]
                .into_iter()
                .collect(),
            &[],
        );
        assert_eq!(matrix.routes("ds-flash").len(), 2);
        assert_eq!(matrix.canonical("ds-flash"), "deepseek-v4.1-flash");
    }

    #[test]
    fn requirements_filter_out_unsuitable_candidates() {
        let matrix = Matrix::build(&catalogs(), &HashMap::new(), &[]);
        let strict = ModelRoute {
            caps_known: true,
            limits: ModelLimits {
                context: Some(1000),
                max_output: Some(100),
            },
            ..matrix.routes("glm-5.3")[0].clone()
        };
        let reason = Matrix::reject_reason(
            &strict,
            &Requirements {
                vision: true,
                tools: false,
                context: 0,
                max_output: None,
            },
        );
        assert!(reason.expect("不支持图片应被拒").contains("图片"));

        // 超输出上限**不**拒绝：上游会钳制，网关照做（见 router 的钳制用例）。
        assert!(Matrix::reject_reason(
            &strict,
            &Requirements {
                vision: false,
                tools: true,
                context: 0,
                max_output: Some(500),
            },
        )
        .is_none());

        let reason = Matrix::reject_reason(
            &strict,
            &Requirements {
                vision: false,
                tools: false,
                context: 5000,
                max_output: None,
            },
        );
        assert!(reason.expect("超上下文应被拒").contains("上下文"));

        assert!(Matrix::reject_reason(&strict, &Requirements::default()).is_none());
    }

    #[test]
    fn unknown_capabilities_do_not_reject_requests() {
        // 快照条目 caps_known=false：不能因为"未知"就拒绝带图片/工具的请求。
        let route = ModelRoute {
            public_model: "hy3".into(),
            channel_id: "wb-cn".into(),
            upstream_model: "hy3".into(),
            priority: 100,
            weight: 1,
            enabled: true,
            credits: None,
            limits: ModelLimits::default(),
            capabilities: ModelCapabilities::default(),
            caps_known: false,
            source: RouteSource::Builtin,
            ..ModelRoute::default()
        };
        let requirements = Requirements {
            vision: true,
            tools: true,
            context: 10,
            max_output: Some(10),
        };
        assert!(Matrix::reject_reason(&route, &requirements).is_none());
    }

    #[test]
    fn overrides_can_disable_prioritise_and_add_routes() {
        let overrides: Vec<RouteOverride> = serde_json::from_value(json!([
            { "model": "deepseek-v4.1-flash", "channel_id": "qoder", "enabled": false },
            { "model": "glm-5.3", "channel_id": "wb-cn", "priority": 7 },
            { "model": "qwen-max", "channel_id": "qoder", "upstream_model": "qwen-max-preview", "priority": 50 }
        ]))
        .expect("覆盖配置");

        let matrix = Matrix::build(&catalogs(), &HashMap::new(), &overrides);
        let ds = matrix.routes("deepseek-v4.1-flash");
        assert_eq!(ds.len(), 1, "被禁用的渠道候选要消失");
        assert_eq!(ds[0].channel_id, "wb-cn");

        let glm = matrix.routes("glm-5.3");
        assert_eq!(glm[0].priority, 7);
        assert_eq!(glm[0].source, RouteSource::Override);

        let qwen = matrix.routes("qwen-max");
        assert_eq!(qwen.len(), 1);
        assert_eq!(qwen[0].upstream_model, "qwen-max-preview", "上游名可以不同");
    }

    #[test]
    fn override_without_upstream_model_cannot_invent_a_candidate() {
        let overrides: Vec<RouteOverride> = serde_json::from_value(json!([
            { "model": "not-in-catalog", "channel_id": "qoder" }
        ]))
        .expect("覆盖配置");
        let matrix = Matrix::build(&catalogs(), &HashMap::new(), &overrides);
        assert!(matrix.routes("not-in-catalog").is_empty());
    }

    #[test]
    fn internal_models_never_reach_the_matrix() {
        let catalogs = vec![ChannelCatalog {
            channel_id: "wb-cn".into(),
            priority: 1,
            weight: 1,
            live: true,
            models: vec![model("lite", None, None), model("auto", None, None)],
        }];
        let matrix = Matrix::build(&catalogs, &HashMap::new(), &[]);
        assert_eq!(matrix.route_count(), 0);
    }
}
