/// Token budget configuration — controls how the context window is partitioned.
#[derive(Debug, Clone)]
pub struct TokenBudget {
    pub max_context: u32,
    pub partitions: TokenPartition,
    pub response_reserve: u32,
}

/// How the context window budget is divided.
#[derive(Debug, Clone)]
pub struct TokenPartition {
    pub system_fraction: f32,
    pub history_fraction: f32,
    pub tool_fraction: f32,
}

impl Default for TokenPartition {
    fn default() -> Self {
        Self {
            system_fraction: 0.15,
            history_fraction: 0.40,
            tool_fraction: 0.30,
        }
    }
}

impl TokenBudget {
    pub fn for_model(max_context: u32) -> Self {
        let response_reserve = (max_context as f32 * 0.15) as u32;
        Self {
            max_context,
            partitions: TokenPartition::default(),
            response_reserve,
        }
    }

    pub fn available(&self) -> u32 {
        self.max_context.saturating_sub(self.response_reserve)
    }

    pub fn system_budget(&self) -> u32 {
        (self.available() as f32 * self.partitions.system_fraction) as u32
    }

    pub fn history_budget(&self) -> u32 {
        (self.available() as f32 * self.partitions.history_fraction) as u32
    }

    pub fn tool_budget(&self) -> u32 {
        (self.available() as f32 * self.partitions.tool_fraction) as u32
    }
}

/// Pressure level for an agent's context window.
///
/// Thresholds align with the `token_budget.pressure` block in agent YAMLs
/// (ADR-2603281000).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PressureLevel {
    /// < 70% — normal operation.
    Ok,
    /// 70–79% — emit inbox warning, agent continues.
    Warn,
    /// 80–89% — trigger prompt compression before next call.
    Compress,
    /// >= 90% — block next inference call, await context relief.
    Block,
}

/// Snapshot of context window pressure for one agent session.
///
/// Updated after every inference call using `input_tokens` returned by the
/// provider. The pressure percentage is a running estimate: it accumulates
/// `input_tokens` across calls, which over-counts for multi-turn sessions
/// (prior turns are re-sent) but provides a conservative upper bound.
#[derive(Debug, Clone)]
pub struct ContextPressure {
    pub session_id: String,
    /// Model context window size in tokens. Provider-specific; defaults are
    /// applied when not explicitly configured.
    pub model_context_limit: u32,
    /// Cumulative input tokens observed across calls in this session.
    pub estimated_used_tokens: u32,
    /// `estimated_used_tokens / model_context_limit * 100`, clamped to 0–100.
    pub pressure_pct: f32,
}

impl ContextPressure {
    pub fn new(session_id: impl Into<String>, model_context_limit: u32) -> Self {
        Self {
            session_id: session_id.into(),
            model_context_limit,
            estimated_used_tokens: 0,
            pressure_pct: 0.0,
        }
    }

    /// Record input tokens from a completed inference call and recompute pressure.
    pub fn record(&mut self, input_tokens: u32) {
        // Use the latest input_tokens as the estimate — for multi-turn sessions
        // each call re-sends the full history, so the latest call's input_tokens
        // is the best single-call estimate of current context size.
        self.estimated_used_tokens = input_tokens;
        self.pressure_pct = if self.model_context_limit > 0 {
            ((input_tokens as f32 / self.model_context_limit as f32) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
    }

    pub fn level(&self) -> PressureLevel {
        if self.pressure_pct >= 90.0 {
            PressureLevel::Block
        } else if self.pressure_pct >= 80.0 {
            PressureLevel::Compress
        } else if self.pressure_pct >= 70.0 {
            PressureLevel::Warn
        } else {
            PressureLevel::Ok
        }
    }
}

/// Tracks actual token usage from API responses.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_input: u64,
    pub total_output: u64,
    pub api_calls: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
}

impl TokenUsage {
    pub fn record(&mut self, input: u32, output: u32) {
        self.input_tokens = input;
        self.output_tokens = output;
        self.total_input += input as u64;
        self.total_output += output as u64;
        self.api_calls += 1;
    }

    pub fn record_with_cache(
        &mut self,
        input: u32,
        output: u32,
        cache_read: u32,
        cache_write: u32,
    ) {
        self.record(input, output);
        self.cache_read_tokens = cache_read;
        self.cache_write_tokens = cache_write;
    }

    pub fn total_tokens(&self) -> u64 {
        self.total_input + self.total_output
    }

    pub fn billable_input(&self) -> u32 {
        self.input_tokens.saturating_sub(self.cache_read_tokens)
    }
}
