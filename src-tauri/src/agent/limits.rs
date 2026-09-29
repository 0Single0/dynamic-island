use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStopReason {
    MaxSteps,
    MaxDuration,
    MaxTokens,
    MaxToolCalls,
    MaxSearchCalls,
    MaxComputerActions,
    EstimatedCost,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunBudget {
    pub max_steps: u32,
    pub max_duration: Duration,
    pub max_total_tokens: u64,
    pub max_tool_calls: u32,
    pub max_search_calls: u32,
    pub max_computer_actions: u32,
    pub max_estimated_cost: Option<f64>,
}

impl Default for RunBudget {
    fn default() -> Self {
        Self {
            max_steps: 12,
            max_duration: Duration::from_secs(900),
            max_total_tokens: 100_000,
            max_tool_calls: 32,
            max_search_calls: 8,
            max_computer_actions: 20,
            max_estimated_cost: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BudgetUsage {
    pub steps: u32,
    pub total_tokens: u64,
    pub tool_calls: u32,
    pub search_calls: u32,
    pub computer_actions: u32,
    pub estimated_cost: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetGuard {
    budget: RunBudget,
    usage: BudgetUsage,
    started_at: Instant,
}

impl BudgetGuard {
    pub fn new(budget: RunBudget) -> Self {
        Self {
            budget,
            usage: BudgetUsage::default(),
            started_at: Instant::now(),
        }
    }

    pub fn usage(&self) -> BudgetUsage {
        self.usage
    }

    pub fn check(&self) -> Result<(), BudgetStopReason> {
        if self.usage.steps >= self.budget.max_steps {
            return Err(BudgetStopReason::MaxSteps);
        }
        if self.started_at.elapsed() >= self.budget.max_duration {
            return Err(BudgetStopReason::MaxDuration);
        }
        if self.usage.total_tokens >= self.budget.max_total_tokens {
            return Err(BudgetStopReason::MaxTokens);
        }
        if self.usage.tool_calls >= self.budget.max_tool_calls {
            return Err(BudgetStopReason::MaxToolCalls);
        }
        if self.usage.search_calls >= self.budget.max_search_calls {
            return Err(BudgetStopReason::MaxSearchCalls);
        }
        if self.usage.computer_actions >= self.budget.max_computer_actions {
            return Err(BudgetStopReason::MaxComputerActions);
        }
        if self
            .budget
            .max_estimated_cost
            .is_some_and(|limit| self.usage.estimated_cost >= limit)
        {
            return Err(BudgetStopReason::EstimatedCost);
        }
        Ok(())
    }

    pub fn begin_step(&mut self) -> Result<(), BudgetStopReason> {
        self.check()?;
        self.usage.steps += 1;
        Ok(())
    }

    pub fn record_model_usage(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cost: f64,
    ) -> Result<(), BudgetStopReason> {
        self.usage.total_tokens = self
            .usage
            .total_tokens
            .saturating_add(input_tokens)
            .saturating_add(output_tokens);
        self.usage.estimated_cost += cost.max(0.0);
        self.check()
    }

    pub fn record_tool(
        &mut self,
        search: bool,
        computer_action: bool,
    ) -> Result<(), BudgetStopReason> {
        self.usage.tool_calls = self.usage.tool_calls.saturating_add(1);
        if search {
            self.usage.search_calls = self.usage.search_calls.saturating_add(1);
        }
        if computer_action {
            self.usage.computer_actions = self.usage.computer_actions.saturating_add(1);
        }
        self.check()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    Never,
    RetryWithBackoff,
    CompactThenRetry,
    ModelFallback,
}

pub fn classify_retry(status: Option<u16>, context_overflow: bool, cancelled: bool) -> RetryClass {
    if cancelled || status == Some(401) || status == Some(403) || status == Some(400) {
        return RetryClass::Never;
    }
    if context_overflow {
        return RetryClass::CompactThenRetry;
    }
    if matches!(
        status,
        Some(408 | 409 | 425 | 429 | 500 | 502 | 503 | 504) | None
    ) {
        return RetryClass::RetryWithBackoff;
    }
    RetryClass::ModelFallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_budget_counter_is_enforced_before_next_operation() {
        let mut guard = BudgetGuard::new(RunBudget {
            max_steps: 1,
            max_tool_calls: 1,
            max_search_calls: 1,
            max_computer_actions: 1,
            max_total_tokens: 10,
            ..RunBudget::default()
        });
        guard.begin_step().unwrap();
        assert_eq!(guard.begin_step(), Err(BudgetStopReason::MaxSteps));
    }

    #[test]
    fn tool_categories_share_the_total_tool_budget() {
        let mut guard = BudgetGuard::new(RunBudget {
            max_tool_calls: 1,
            max_search_calls: 1,
            max_computer_actions: 1,
            ..RunBudget::default()
        });
        guard.record_tool(true, false).unwrap_err();
        assert_eq!(guard.usage().tool_calls, 1);
        assert_eq!(guard.usage().search_calls, 1);
        assert_eq!(
            guard.record_tool(false, false),
            Err(BudgetStopReason::MaxToolCalls)
        );
    }

    #[test]
    fn retry_policy_does_not_retry_auth_or_cancel() {
        assert_eq!(classify_retry(Some(401), false, false), RetryClass::Never);
        assert_eq!(
            classify_retry(Some(500), false, false),
            RetryClass::RetryWithBackoff
        );
        assert_eq!(
            classify_retry(None, true, false),
            RetryClass::CompactThenRetry
        );
        assert_eq!(classify_retry(Some(500), false, true), RetryClass::Never);
    }
}
