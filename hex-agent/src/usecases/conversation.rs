use crate::domain::{
    ApiRequestOptions, ContentBlock, ConversationState, Message, Role,
    ThinkingConfig, TokenBudget, ToolCall, ToolDefinition, WorkloadClass,
};
use crate::ports::anthropic::{AnthropicError, AnthropicPort};
use crate::ports::context::ContextManagerPort;
use crate::ports::conversation::{ConversationEvent, ConversationError, ConversationPort};
use crate::ports::rate_limiter::RateLimiterPort;
use crate::ports::rl::{ContextStrategy, ModelSelection, RlPort, RlReward, RlState};
use crate::ports::token_metrics::TokenMetricsPort;
use crate::ports::tools::ToolExecutorPort;
use async_trait::async_trait;
use std::sync::Arc;

/// The core conversation loop — multi-turn interaction with tool_use support.
///
/// This is the use case that replaces Claude Code's internal agent loop.
/// It orchestrates: RL query → rate limit check → context packing → API call
/// (with caching + thinking) → tool execution → metrics recording → reward.
pub struct ConversationLoop {
    anthropic: Arc<dyn AnthropicPort>,
    context_mgr: Arc<dyn ContextManagerPort>,
    tool_executor: Arc<dyn ToolExecutorPort>,
    rl: Arc<dyn RlPort>,
    rate_limiter: Arc<dyn RateLimiterPort>,
    metrics: Arc<dyn TokenMetricsPort>,
    tools: Vec<ToolDefinition>,
    budget: TokenBudget,
    max_response_tokens: u32,
    max_tool_rounds: u32,
    /// When `true`, the user pinned a model via `--model` and RL must not override it.
    model_pinned: bool,
    /// Enable prompt caching for system + tool blocks.
    enable_cache: bool,
    /// Extended thinking budget (0 = disabled).
    thinking_budget: u32,
}

impl ConversationLoop {
    pub fn new(
        anthropic: Arc<dyn AnthropicPort>,
        context_mgr: Arc<dyn ContextManagerPort>,
        tool_executor: Arc<dyn ToolExecutorPort>,
        rl: Arc<dyn RlPort>,
        rate_limiter: Arc<dyn RateLimiterPort>,
        metrics: Arc<dyn TokenMetricsPort>,
        tools: Vec<ToolDefinition>,
        budget: TokenBudget,
        max_response_tokens: u32,
    ) -> Self {
        Self {
            anthropic,
            context_mgr,
            tool_executor,
            rl,
            rate_limiter,
            metrics,
            tools,
            budget,
            max_response_tokens,
            max_tool_rounds: 25,
            model_pinned: false,
            enable_cache: true,   // On by default — free savings
            thinking_budget: 0,
        }
    }

    /// Mark the model as user-pinned (via `--model` CLI arg).
    /// When pinned, RL model selection is ignored — the configured model is always used.
    pub fn with_model_pinned(mut self, pinned: bool) -> Self {
        self.model_pinned = pinned;
        self
    }

    /// Set the extended thinking budget.
    pub fn with_thinking_budget(mut self, budget: u32) -> Self {
        self.thinking_budget = budget;
        self
    }

    /// Enable/disable prompt caching.
    pub fn with_cache(mut self, enabled: bool) -> Self {
        self.enable_cache = enabled;
        self
    }

    /// Fallback chain for rate-limited models.
    /// Returns the next model to try after `current`, or `None` if exhausted.
    fn fallback_model(current: &ModelSelection) -> Option<ModelSelection> {
        match current {
            ModelSelection::Opus => Some(ModelSelection::Sonnet),
            ModelSelection::Sonnet => Some(ModelSelection::Haiku),
            ModelSelection::Haiku => Some(ModelSelection::Local),
            ModelSelection::Local => None,
        }
    }

    /// Process a single user message through the full conversation loop.
    ///
    /// This handles multi-round tool_use: if the model responds with tool calls,
    /// we execute them and continue until the model produces a text-only response
    /// or we hit the tool round limit.
    async fn run_turn(
        &self,
        state: &mut ConversationState,
        user_input: &str,
        event_tx: &tokio::sync::mpsc::UnboundedSender<ConversationEvent>,
    ) -> Result<(), ConversationError> {
        // Query RL engine for optimal context strategy + model selection
        let rl_state = RlState {
            task_type: "conversation".to_string(),
            codebase_size: 0, // TODO: inject from project metadata
            agent_count: 1,
            token_usage: state.total_estimated_tokens() as u64,
            ..Default::default()
        };
        let rl_action = self.rl.select_action(&rl_state).await.ok();
        let strategy = rl_action
            .as_ref()
            .map(|a| a.context_strategy.clone())
            .unwrap_or(ContextStrategy::Balanced);

        // RL model selection — only when the user hasn't pinned a model via --model
        let rl_model: Option<ModelSelection> = if !self.model_pinned {
            rl_action.as_ref().map(|a| a.model.clone())
        } else {
            None
        };

        if let Some(ref model) = rl_model {
            tracing::info!(model = %model.model_id(), "RL selected model");
        }

        // Adjust budget based on RL-selected strategy
        let mut budget = self.budget.clone();
        budget.partitions.history_fraction *= strategy.history_multiplier();
        budget.partitions.tool_fraction *= strategy.tool_multiplier();

        if strategy != ContextStrategy::Balanced {
            tracing::info!(
                strategy = ?strategy,
                action = rl_action.as_ref().map(|a| a.action.as_str()).unwrap_or("default"),
                "RL selected context strategy"
            );
        }

        // Add user message
        state.push(Message::user(user_input));

        let mut tool_rounds = 0;
        let turn_start_tokens = state.total_estimated_tokens();
        let turn_start_time = std::time::Instant::now();
        let mut was_rate_limited = false;
        #[allow(unused_assignments)] // always overwritten inside the loop before being read
        let mut actual_model_used = String::new();

        // Build API request options with caching + thinking config
        let api_options = ApiRequestOptions {
            enable_cache: self.enable_cache,
            thinking: ThinkingConfig::with_budget(self.thinking_budget),
            workload: Some(WorkloadClass::Interactive),
        };

        loop {
            // Pack context to fit within RL-adjusted budget
            let packed = self
                .context_mgr
                .pack(state, &budget)
                .await
                .map_err(|e| ConversationError::ContextError(e.to_string()))?;

            if packed.evicted_count > 0 {
                tracing::info!(evicted = packed.evicted_count, "Context window trimmed");
            }

            // Send to Anthropic API with RL-driven model selection and fallback
            let response = {
                let mut current_model = rl_model.clone();
                let mut attempt = 0u32;
                loop {
                    let override_id = current_model
                        .as_ref()
                        .map(|m| m.model_id().to_string());

                    // Proactive rate limit check — wait if we're near the limit
                    let model_id = override_id.as_deref().unwrap_or("default");
                    if let Some(delay) = self
                        .rate_limiter
                        .should_throttle(
                            model_id,
                            packed.messages.iter().map(|m| m.estimated_tokens()).sum::<u32>(),
                            self.max_response_tokens,
                        )
                        .await
                    {
                        tracing::info!(
                            model = model_id,
                            delay_ms = delay.as_millis() as u64,
                            "Proactive throttle — waiting to avoid rate limit"
                        );
                        let _ = event_tx.send(ConversationEvent::TextChunk(
                            format!("\n*Throttling {}ms to stay within rate limits...*\n", delay.as_millis()),
                        ));
                        tokio::time::sleep(delay).await;
                    }

                    match self
                        .anthropic
                        .send_message(
                            &packed.system_prompt,
                            &packed.messages,
                            &self.tools,
                            self.max_response_tokens,
                            override_id.as_deref(),
                            Some(&api_options),
                        )
                        .await
                    {
                        Ok(resp) => {
                            actual_model_used = resp.model.clone();

                            // Record usage with rate limiter + metrics
                            self.rate_limiter
                                .record_usage(
                                    model_id,
                                    resp.usage.input_tokens,
                                    resp.usage.output_tokens,
                                )
                                .await;
                            self.metrics
                                .record_realtime(
                                    model_id,
                                    resp.usage.input_tokens,
                                    resp.usage.output_tokens,
                                    resp.usage.cache_read_tokens,
                                    resp.usage.cache_write_tokens,
                                )
                                .await;

                            if resp.usage.cache_read_tokens > 0 {
                                tracing::debug!(
                                    cache_read = resp.usage.cache_read_tokens,
                                    cache_write = resp.usage.cache_write_tokens,
                                    "Prompt cache hit"
                                );
                            }

                            break resp;
                        }
                        Err(AnthropicError::RateLimited { retry_after_ms }) => {
                            was_rate_limited = true;

                            // Record rate limit with the rate limiter
                            self.rate_limiter.record_rate_limit(model_id).await;

                            let rate_limited_model = current_model
                                .as_ref()
                                .map(|m| m.model_id())
                                .unwrap_or("default");

                            // Report negative reward for the rate-limited model
                            if let Some(ref action) = rl_action {
                                let _ = self
                                    .rl
                                    .report_reward(&RlReward {
                                        state_key: action.state_key.clone(),
                                        action: format!("model:{}", rate_limited_model),
                                        reward: -0.5,
                                        next_state_key: action.state_key.clone(),
                                        rate_limited: true,
                                        model_used: rate_limited_model.to_string(),
                                        latency_ms: turn_start_time.elapsed().as_millis() as u64,
                                    })
                                    .await;
                            }

                            // Try fallback model if available and model isn't pinned
                            if !self.model_pinned {
                                if let Some(ref cm) = current_model {
                                    if let Some(fallback) = Self::fallback_model(cm) {
                                        tracing::warn!(
                                            from = %cm.model_id(),
                                            to = %fallback.model_id(),
                                            "Rate limited — falling back to next model"
                                        );
                                        let _ = event_tx.send(ConversationEvent::TextChunk(
                                            format!(
                                                "\n*Rate limited on {} — trying {}...*\n",
                                                cm.model_id(),
                                                fallback.model_id()
                                            ),
                                        ));
                                        current_model = Some(fallback);
                                        continue;
                                    }
                                }
                            }

                            // No fallback available or model is pinned — retry with backoff
                            attempt += 1;
                            if attempt > 3 {
                                return Err(ConversationError::ApiError(
                                    format!("Rate limited after {} retries", attempt),
                                ));
                            }
                            let wait = std::cmp::min(retry_after_ms, 60_000);
                            tracing::warn!(
                                attempt,
                                wait_ms = wait,
                                "Rate limited — retrying in {}s",
                                wait / 1000
                            );
                            let _ = event_tx.send(ConversationEvent::TextChunk(
                                format!("\n*Rate limited — retrying in {}s...*\n", wait / 1000),
                            ));
                            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                        }
                        Err(e) => {
                            return Err(ConversationError::ApiError(e.to_string()));
                        }
                    }
                }
            };

            // Report token usage
            let _ = event_tx.send(ConversationEvent::TokenUpdate(response.usage.clone()));

            // Process the response content blocks
            let mut has_tool_use = false;
            let mut tool_results: Vec<ContentBlock> = Vec::new();
            let mut assistant_content: Vec<ContentBlock> = Vec::new();

            for block in &response.content {
                match block {
                    ContentBlock::Text { text } => {
                        let _ = event_tx.send(ConversationEvent::TextChunk(text.clone()));
                        assistant_content.push(block.clone());
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        has_tool_use = true;
                        assistant_content.push(block.clone());

                        let input_str = serde_json::to_string_pretty(input)
                            .unwrap_or_else(|_| input.to_string());
                        let _ = event_tx.send(ConversationEvent::ToolCallStart {
                            name: name.clone(),
                            input: input_str,
                        });

                        // Execute the tool
                        let call = ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        };
                        let result = self.tool_executor.execute(&call).await;

                        let _ = event_tx.send(ConversationEvent::ToolCallResult {
                            name: name.clone(),
                            content: result.content.clone(),
                            is_error: result.is_error,
                        });

                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: result.tool_use_id,
                            content: result.content,
                            is_error: Some(result.is_error),
                        });
                    }
                    _ => {}
                }
            }

            // Add assistant message to state
            state.push(Message {
                role: Role::Assistant,
                content: assistant_content,
            });
            state.last_stop_reason = Some(response.stop_reason.clone());

            // If there were tool calls, add results and continue the loop
            if has_tool_use && !tool_results.is_empty() {
                state.push(Message {
                    role: Role::User,
                    content: tool_results,
                });

                tool_rounds += 1;
                if tool_rounds >= self.max_tool_rounds {
                    let _ = event_tx.send(ConversationEvent::Error(format!(
                        "Tool round limit reached ({})",
                        self.max_tool_rounds
                    )));
                    break;
                }

                // Continue the loop — model needs to process tool results
                continue;
            }

            // No tool calls or end_turn — conversation turn is complete
            let _ = event_tx.send(ConversationEvent::TurnComplete {
                stop_reason: response.stop_reason,
            });
            break;
        }

        // Report reward to RL engine with model selection + latency signals
        if let Some(ref action) = rl_action {
            let turn_end_tokens = state.total_estimated_tokens();
            let latency_ms = turn_start_time.elapsed().as_millis() as u64;
            let tokens_used = turn_end_tokens.saturating_sub(turn_start_tokens) as f64;

            // Reward formula:
            //   success(+1.0) + fast_bonus(if <2s: +0.2) - rate_limit_penalty(if 429: -0.5) - token_cost(tokens/10000)
            let success_score = 1.0;
            let fast_bonus = if latency_ms < 2000 { 0.2 } else { 0.0 };
            let rate_limit_penalty = if was_rate_limited { -0.5 } else { 0.0 };
            let token_cost = -(tokens_used / 10000.0);
            let tool_penalty = if tool_rounds >= self.max_tool_rounds {
                -0.5
            } else {
                0.0
            };
            let reward = success_score + fast_bonus + rate_limit_penalty + token_cost + tool_penalty;

            // Best-effort — don't fail the turn if reward reporting fails
            let next_key = format!(
                "conversation:sz0:ag1:tk{}",
                match turn_end_tokens {
                    0..=1000 => 0,
                    1001..=10000 => 1,
                    10001..=50000 => 2,
                    50001..=200000 => 3,
                    _ => 4,
                }
            );
            let _ = self
                .rl
                .report_reward(&RlReward {
                    state_key: action.state_key.clone(),
                    action: action.action.clone(),
                    reward,
                    next_state_key: next_key,
                    rate_limited: was_rate_limited,
                    model_used: actual_model_used.clone(),
                    latency_ms,
                })
                .await;

            tracing::debug!(
                reward = reward,
                tokens_used = tokens_used,
                tool_rounds = tool_rounds,
                model = %actual_model_used,
                latency_ms = latency_ms,
                rate_limited = was_rate_limited,
                "RL reward reported"
            );
        }

        Ok(())
    }
}

use crate::ports::conversation::ConversationCheckpoint;

#[async_trait]
impl ConversationPort for ConversationLoop {
    async fn process_message(
        &self,
        state: &mut ConversationState,
        user_input: &str,
        event_tx: &tokio::sync::mpsc::UnboundedSender<ConversationEvent>,
    ) -> Result<(), ConversationError> {
        self.run_turn(state, user_input, event_tx).await
    }

    async fn reset_context(
        &self,
        state: &mut ConversationState,
        new_system_prompt: Option<String>,
    ) -> Result<ConversationCheckpoint, ConversationError> {
        // 1. Build summary from current conversation
        let summary = if state.messages.is_empty() {
            "Empty conversation".to_string()
        } else {
            let msg_count = state.messages.len();
            let last_text = state.messages.last()
                .map(|m| m.text_content())
                .unwrap_or_default();
            let truncated = if last_text.len() > 200 {
                format!("{}...", &last_text[..200])
            } else {
                last_text
            };
            format!(
                "Conversation with {} messages, {} turns. Last: {}",
                msg_count, state.turn_count, truncated
            )
        };

        // 2. Create checkpoint
        let checkpoint = ConversationCheckpoint {
            conversation_id: state.conversation_id.clone(),
            turn_count: state.turn_count,
            summary: summary.clone(),
            total_input_tokens: state.total_estimated_tokens() as u64,
            total_output_tokens: 0,
        };

        // 3. Clear message history
        state.messages.clear();
        state.turn_count = 0;
        state.last_stop_reason = None;

        // 4. Inject new system prompt if provided
        if let Some(prompt) = new_system_prompt {
            state.system_prompt = prompt;
        }

        // 5. Generate new conversation ID for the fresh context
        state.conversation_id = uuid::Uuid::new_v4().to_string();

        tracing::info!(
            old_turns = checkpoint.turn_count,
            new_id = %state.conversation_id,
            "Context reset — fresh window for new task"
        );

        Ok(checkpoint)
    }
}
