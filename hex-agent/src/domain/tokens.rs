/// Token budget configuration — controls how the context window is partitioned.
///
/// The Anthropic Messages API has a fixed context window (e.g., 200k tokens).
/// We partition it into zones to prevent any single category from starving others.
#[derive(Debug, Clone)]
pub struct TokenBudget {
    /// Total tokens available in the model's context window
    pub max_context: u32,
    /// How to divide the budget across categories
    pub partitions: TokenPartition,
    /// Tokens reserved for the model's response
    pub response_reserve: u32,
}

/// How the context window budget is divided.
///
/// Percentages of (max_context - response_reserve):
/// - system: CLAUDE.md, dependency graph, workplan state, skill manifests
/// - history: conversation turns
/// - tools: tool_use results (can be large — file contents, grep output)
#[derive(Debug, Clone)]
pub struct TokenPartition {
    /// Fraction for system prompt (0.0-1.0)
    pub system_fraction: f32,
    /// Fraction for conversation history (0.0-1.0)
    pub history_fraction: f32,
    /// Fraction for tool results (0.0-1.0)
    pub tool_fraction: f32,
}

impl Default for TokenPartition {
    fn default() -> Self {
        Self {
            system_fraction: 0.15,
            history_fraction: 0.40,
            tool_fraction: 0.30,
            // Remaining 0.15 is response_reserve (handled separately)
        }
    }
}

impl TokenBudget {
    /// Create a budget for a given model context size.
    pub fn for_model(max_context: u32) -> Self {
        let response_reserve = (max_context as f32 * 0.15) as u32;
        Self {
            max_context,
            partitions: TokenPartition::default(),
            response_reserve,
        }
    }

    /// Available tokens after subtracting response reserve.
    pub fn available(&self) -> u32 {
        self.max_context.saturating_sub(self.response_reserve)
    }

    /// Max tokens for system prompt.
    pub fn system_budget(&self) -> u32 {
        (self.available() as f32 * self.partitions.system_fraction) as u32
    }

    /// Max tokens for conversation history.
    pub fn history_budget(&self) -> u32 {
        (self.available() as f32 * self.partitions.history_fraction) as u32
    }

    /// Max tokens for tool results.
    pub fn tool_budget(&self) -> u32 {
        (self.available() as f32 * self.partitions.tool_fraction) as u32
    }
}

/// Tracks actual token usage from API responses.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Cumulative across the full conversation
    pub total_input: u64,
    pub total_output: u64,
    /// Number of API calls made
    pub api_calls: u32,
    /// Tokens served from prompt cache (free, bypasses input TPM)
    pub cache_read_tokens: u32,
    /// Tokens written to prompt cache (1.25x cost on first request)
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

    /// Record with cache breakdown from API response.
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

    /// Effective input tokens (excluding cached reads which are free).
    pub fn billable_input(&self) -> u32 {
        self.input_tokens.saturating_sub(self.cache_read_tokens)
    }
}
