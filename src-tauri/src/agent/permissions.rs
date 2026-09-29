use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiskLevel {
    L0,
    L1,
    L2,
    L3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPolicy {
    pub allow_l0: bool,
    pub allow_l1: bool,
    pub require_confirmation_l2: bool,
    pub require_confirmation_l3: bool,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self {
            allow_l0: true,
            allow_l1: true,
            require_confirmation_l2: true,
            require_confirmation_l3: true,
        }
    }
}

impl PermissionPolicy {
    pub fn requires_confirmation(&self, risk: RiskLevel) -> bool {
        match risk {
            RiskLevel::L0 => !self.allow_l0,
            RiskLevel::L1 => !self.allow_l1,
            RiskLevel::L2 => self.require_confirmation_l2,
            RiskLevel::L3 => self.require_confirmation_l3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalSnapshot {
    pub id: String,
    pub run_id: String,
    pub tool_name: String,
    pub args_json: String,
    pub args_hash: String,
    pub risk: RiskLevel,
    pub expires_at: String,
    pub status: String,
}

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
    fn approval_hash_is_stable_and_policy_defaults_to_confirm() {
        assert_eq!(args_hash("{}"), args_hash("{}"));
        assert!(!PermissionPolicy::default().requires_confirmation(RiskLevel::L1));
        assert!(PermissionPolicy::default().requires_confirmation(RiskLevel::L2));
    }
}
