use serde::{Deserialize, Serialize};

/// 工具风险等级：只用于界面徽章与轨迹标注，不再驱动任何执行前的确认。
///
/// Agent 现在是全自主执行，安全由技术护栏（路径/SSRF/目标窗口校验、token 与步数预算）
/// 和事后审计（`agent_events`）保证，而不是执行前的人工批准。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiskLevel {
    L0,
    L1,
    L2,
    L3,
}

/// 工具参数的稳定哈希（键排序后计算），用于轨迹里核对同一次调用。
pub fn args_hash(args_json: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    args_json.hash(&mut hasher);
    format!("args:{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approval_hash_is_stable() {
        assert_eq!(args_hash("{}"), args_hash("{}"));
        assert_ne!(args_hash("{}"), args_hash("{\"a\":1}"));
    }
}
