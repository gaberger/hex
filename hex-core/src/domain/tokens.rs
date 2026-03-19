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
