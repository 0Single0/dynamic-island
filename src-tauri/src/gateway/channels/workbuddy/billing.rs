//! 计费侧额度查询（国内版 = 积分 / 资源包）。
//!
//! 三步（与参考实现一致）：
//! ① `get-user-resource-summary` 拿资源包编号与是否付费用户；
//! ② `get-user-resource-free-packages` / `get-user-resource-paid-packages` 拿每个包的
//!    周期容量（Size / Remain / Used，都有 Precise 变体，两种都要兼容）；
//! ③ 汇总成渠道无关的 [`QuotaSnapshot`]，面板只按 label + unit 展示。
//!
//! 已知简化：企业账号在个人计费接口上拿不到资源包（全是 0），
//! 这里只标注一下，等真需要企业用量时再接 `get-enterprise-user-usage`。

use serde_json::{json, Value};

use crate::gateway::{
    channels::{AccountSnapshot, QuotaSnapshot},
    errors::GatewayError,
    timeutil::now_ms,
    transport,
};

use super::identity::{self, Hosts};

const SUMMARY_PATH: &str = "/billing/meter/get-user-resource-summary";
const FREE_PATH: &str = "/billing/meter/get-user-resource-free-packages";
const PAID_PATH: &str = "/billing/meter/get-user-resource-paid-packages";

pub async fn fetch(
    client: &reqwest::Client,
    hosts: &Hosts,
    account: &AccountSnapshot,
) -> Result<Option<QuotaSnapshot>, GatewayError> {
    // 计费头组比对话头更轻：带 X-Client-Platform: web，无企业身份时显式声明。
    let headers = identity::billing_headers(hosts, account, Some("web"));
    let summary = transport::post_json(client, &hosts.billing_url(SUMMARY_PATH), &headers, &json!({}))
        .await?;
    if summary.get("code").and_then(Value::as_i64) != Some(0) {
        return Ok(None);
    }
    let data = summary.get("data").cloned().unwrap_or(Value::Null);
    let codes: Vec<String> = data
        .get("Packages")
        .and_then(Value::as_array)
        .map(|packages| {
            packages
                .iter()
                .filter_map(|package| {
                    package
                        .get("PackageCode")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    let paid_user = data
        .get("IsPaidUser")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut total = 0.0f64;
    let mut remaining = 0.0f64;
    let mut used = 0.0f64;
    let mut unit = String::new();
    let mut free_count = 0usize;
    let mut paid_count = 0usize;

    for (path, status, need_renew, is_free) in [
        (FREE_PATH, json!([0]), false, true),
        (PAID_PATH, json!([0, 3]), true, false),
    ] {
        let body = json!({
            "PackageCodes": codes,
            "PageNumber": 1,
            "PageSize": 200,
            "Status": status,
            "NeedRenewInfo": need_renew,
        });
        let Ok(payload) = transport::post_json(client, &hosts.billing_url(path), &headers, &body).await
        else {
            // 某类包查不到不影响整体：有哪类就报哪类。
            continue;
        };
        for package in collect_packages(&payload) {
            let (size, remain, spent) = capacities(package);
            if size <= 0.0 && remain <= 0.0 {
                continue;
            }
            total += size;
            remaining += remain;
            used += spent;
            if unit.is_empty() {
                if let Some(value) = package.get("CapacityUnit").and_then(Value::as_str) {
                    if !value.is_empty() {
                        unit = value.to_string();
                    }
                }
            }
            if is_free {
                free_count += 1;
            } else {
                paid_count += 1;
            }
        }
    }

    let mut details = vec![
        ("免费资源包".to_string(), format!("{free_count} 个")),
        ("付费资源包".to_string(), format!("{paid_count} 个")),
    ];
    if paid_user {
        details.push(("账号类型".to_string(), "付费用户".to_string()));
    }
    if free_count == 0 && paid_count == 0 {
        details.push((
            "说明".to_string(),
            "个人计费接口没有资源包；企业账号请在企业用量里查看".to_string(),
        ));
    }

    Ok(Some(QuotaSnapshot {
        label: "积分".to_string(),
        unit: if unit.is_empty() {
            "credits".to_string()
        } else {
            unit
        },
        remaining: Some(round2(remaining)),
        total: (total > 0.0).then(|| round2(total)),
        used: (used > 0.0).then(|| round2(used)),
        details,
        fetched_at_ms: now_ms(),
    }))
}

/// 在响应里**递归**找资源包条目：免费/付费两个接口的容器层级不完全一致，
/// 按"含容量字段的对象"识别比猜字段名稳。
fn collect_packages(payload: &Value) -> Vec<&Value> {
    let mut found = Vec::new();
    fn walk<'a>(node: &'a Value, found: &mut Vec<&'a Value>) {
        match node {
            Value::Object(map) => {
                let looks_like_package = map.keys().any(|key| {
                    key.starts_with("CycleCapacity") || key.starts_with("Capacity")
                }) || map.contains_key("PackageCode");
                if looks_like_package && !map.is_empty() {
                    found.push(node);
                }
                for value in map.values() {
                    walk(value, found);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, found);
                }
            }
            _ => {}
        }
    }
    walk(payload, &mut found);
    found
}

/// 容量三件套：优先 Precise 变体；缺一个就用另外两个补（参考实现同款修正）。
fn capacities(package: &Value) -> (f64, f64, f64) {
    let value = |keys: [&str; 4]| -> f64 {
        for key in keys {
            if let Some(raw) = package.get(key) {
                let parsed = match raw {
                    Value::Number(number) => number.as_f64(),
                    Value::String(text) => text.parse::<f64>().ok(),
                    _ => None,
                };
                if let Some(parsed) = parsed {
                    return parsed;
                }
            }
        }
        0.0
    };
    let size = value([
        "CycleCapacitySizePrecise",
        "CycleCapacitySize",
        "CapacitySizePrecise",
        "CapacitySize",
    ]);
    let mut remain = value([
        "CycleCapacityRemainPrecise",
        "CycleCapacityRemain",
        "CapacityRemainPrecise",
        "CapacityRemain",
    ]);
    let mut used = value([
        "CycleCapacityUsedPrecise",
        "CycleCapacityUsed",
        "CapacityUsedPrecise",
        "CapacityUsed",
    ]);

    if size > 0.0 && used <= 0.0 && remain <= size {
        used = (size - remain).max(0.0);
    } else if size > 0.0 && remain <= 0.0 && used < size {
        remain = (size - used).max(0.0);
    }
    (size, remain, used)
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_variants_and_fixups_are_handled() {
        // 只有 Size + Remain：Used 要能推出来。
        let package = json!({"CycleCapacitySizePrecise": "1000", "CycleCapacityRemain": 250});
        assert_eq!(capacities(&package), (1000.0, 250.0, 750.0));

        // 只有 Size + Used：Remain 要能推出来。
        let package = json!({"CapacitySize": 100, "CapacityUsedPrecise": 40});
        assert_eq!(capacities(&package), (100.0, 60.0, 40.0));

        // 数字与字符串都要认。
        let package = json!({"CycleCapacitySize": "1.5", "CycleCapacityRemain": "0.5"});
        assert_eq!(capacities(&package), (1.5, 0.5, 1.0));

        assert_eq!(capacities(&json!({})), (0.0, 0.0, 0.0));
    }

    #[test]
    fn packages_are_found_at_any_depth() {
        let payload = json!({
            "code": 0,
            "data": {"Accounts": [
                {"PackageCode": "p1", "CycleCapacitySizePrecise": 10, "CycleCapacityRemainPrecise": 4}
            ]}
        });
        let packages = collect_packages(&payload);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0]["PackageCode"], "p1");

        // 容器换成 Packages 也一样能找到。
        let payload = json!({"data": {"Packages": [{"PackageCode": "p2", "CapacitySize": 5}]}});
        assert_eq!(collect_packages(&payload).len(), 1);
    }
}
