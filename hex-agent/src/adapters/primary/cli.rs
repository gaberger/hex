use crate::ports::{ConversationState, StopReason};
use crate::ports::conversation::{ConversationEvent, ConversationPort};
use std::io::{self, BufRead, Write};
use tokio::sync::mpsc;

/// Interactive CLI adapter — reads from stdin, writes to stdout.
///
/// This is the primary adapter for standalone hex-agent usage
/// (not managed by hex-hub).
pub struct CliAdapter {
    conversation: Box<dyn ConversationPort>,
    system_prompt: Option<String>,
}

impl CliAdapter {
    pub fn new(conversation: Box<dyn ConversationPort>) -> Self {
        Self { conversation, system_prompt: None }
    }

    /// Set the system prompt injected into the conversation state at startup.
    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = Some(prompt);
        self
    }

    /// Run in one-shot mode (prompt provided) or interactive REPL.
    ///
    /// One-shot: if `prompt` is Some, send it as a single message and exit.
    /// Pipe mode: if stdin is not a TTY and prompt is None, read all of stdin as one message.
    /// Interactive: fall through to the REPL loop.
    pub async fn run(&self, prompt: Option<String>) -> anyhow::Result<()> {
        let mut state = ConversationState::new(uuid::Uuid::new_v4().to_string());
        if let Some(ref sp) = self.system_prompt {
            state.system_prompt = sp.clone();
        }

        // One-shot: --prompt flag provided
        if let Some(p) = prompt {
            return self.send_and_exit(&mut state, &p).await;
        }

        // Pipe mode: stdin is not a TTY — read entire stdin as a single message.
        if !atty::is(atty::Stream::Stdin) {
            let buf = io::read_to_string(io::stdin())?;
            let buf = buf.trim().to_string();
            if !buf.is_empty() {
                return self.send_and_exit(&mut state, &buf).await;
            }
            return Ok(());
        }

        eprintln!("hex-agent — type your message, press Enter to send. Ctrl+D to quit.\n");

        let stdin = io::stdin();

        loop {
            // Prompt
            eprint!("\x1b[36m❯\x1b[0m ");
            io::stderr().flush()?;

            // Read user input
            let mut input = String::new();
            if stdin.lock().read_line(&mut input)? == 0 {
                // EOF (Ctrl+D)
                eprintln!("\nGoodbye.");
                break;
            }

            let input = input.trim();
            if input.is_empty() {
                continue;
            }

            // Handle special commands
            if input == "/quit" || input == "/exit" {
                break;
            }
            if input == "/tokens" {
                eprintln!(
                    "Estimated tokens in conversation: {}",
                    state.total_estimated_tokens()
                );
                continue;
            }
            if input == "/clear" {
                state = ConversationState::new(uuid::Uuid::new_v4().to_string());
                eprintln!("Conversation cleared.");
                continue;
            }

            // /plan triggers context reset — fresh window for planning
            let plan_input: String;
            let input = if input.starts_with("/plan ") || input == "/plan" {
                let plan_args = input.strip_prefix("/plan").unwrap_or("").trim();
                match self.conversation.reset_context(&mut state, None).await {
                    Ok(checkpoint) => {
                        eprintln!(
                            "\x1b[35m↻ Context reset (checkpoint: {} turns, id: {})\x1b[0m",
                            checkpoint.turn_count, checkpoint.conversation_id
                        );
                    }
                    Err(e) => {
                        eprintln!("\x1b[33mWarning: context reset failed: {}\x1b[0m", e);
                    }
                }
                // Rewrite as a planning prompt with full budget
                if plan_args.is_empty() {
                    "You are now in planning mode. What would you like to plan?"
                } else {
                    plan_input = format!("Create a hex workplan for: {}", plan_args);
                    plan_input.as_str()
                }
            } else {
                input
            };

            // Process through conversation loop
            let (tx, mut rx) = mpsc::unbounded_channel::<ConversationEvent>();

            // Spawn event consumer that prints to stdout
            let print_handle = tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    match event {
                        ConversationEvent::TextChunk(text) => {
                            print!("{}", text);
                            let _ = io::stdout().flush();
                        }
                        ConversationEvent::ToolCallStart { name, input } => {
                            eprintln!("\n\x1b[33m⚙ {}({})\x1b[0m", name, truncate(&input, 80));
                        }
                        ConversationEvent::ToolCallResult {
                            name,
                            is_error,
                            ..
                        } => {
                            if is_error {
                                eprintln!("\x1b[31m✗ {} failed\x1b[0m", name);
                            } else {
                                eprintln!("\x1b[32m✓ {}\x1b[0m", name);
                            }
                        }
                        ConversationEvent::TokenUpdate(usage) => {
                            tracing::debug!(
                                input = usage.input_tokens,
                                output = usage.output_tokens,
                                "Token usage"
                            );
                        }
                        ConversationEvent::TurnComplete { stop_reason } => {
                            println!();
                            if stop_reason == StopReason::MaxTokens {
                                eprintln!("\x1b[33m⚠ Response truncated (max tokens)\x1b[0m");
                            }
                        }
                        ConversationEvent::ContextReset { summary } => {
                            eprintln!("\x1b[35m↻ Context reset: {}\x1b[0m", summary);
                        }
                        ConversationEvent::Error(msg) => {
                            eprintln!("\x1b[31mError: {}\x1b[0m", msg);
                        }
                    }
                }
            });

            // Run the conversation
            if let Err(e) = self.conversation.process_message(&mut state, input, &tx).await {
                eprintln!("\x1b[31mConversation error: {}\x1b[0m", e);
            }

            // Drop sender to signal the print task to finish
            drop(tx);
            let _ = print_handle.await;
        }

        Ok(())
    }

    /// Send a single message, print the response to stdout, then return.
    async fn send_and_exit(&self, state: &mut ConversationState, prompt: &str) -> anyhow::Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel::<ConversationEvent>();

        let print_handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    ConversationEvent::TextChunk(text) => {
                        print!("{}", text);
                        let _ = io::stdout().flush();
                    }
                    ConversationEvent::TurnComplete { stop_reason } => {
                        println!();
                        if stop_reason == StopReason::MaxTokens {
                            eprintln!("\x1b[33m⚠ Response truncated (max tokens)\x1b[0m");
                        }
                    }
                    ConversationEvent::ToolCallStart { name, input } => {
                        eprintln!("\n\x1b[33m⚙ {}({})\x1b[0m", name, truncate(&input, 80));
                    }
                    ConversationEvent::ToolCallResult { name, is_error, .. } => {
                        if is_error {
                            eprintln!("\x1b[31m✗ {} failed\x1b[0m", name);
                        } else {
                            eprintln!("\x1b[32m✓ {}\x1b[0m", name);
                        }
                    }
                    ConversationEvent::Error(msg) => {
                        eprintln!("\x1b[31mError: {}\x1b[0m", msg);
                    }
                    _ => {}
                }
            }
        });

        if let Err(e) = self.conversation.process_message(state, prompt, &tx).await {
            eprintln!("\x1b[31mConversation error: {}\x1b[0m", e);
        }

        drop(tx);
        let _ = print_handle.await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::conversation::{ConversationCheckpoint, ConversationError, ConversationPort};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    /// A minimal mock that records every `process_message` call.
    struct RecordingConversation {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingConversation {
        fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            (Self { calls: calls.clone() }, calls)
        }
    }

    #[async_trait]
    impl ConversationPort for RecordingConversation {
        async fn process_message(
            &self,
            _state: &mut ConversationState,
            user_input: &str,
            event_tx: &tokio::sync::mpsc::UnboundedSender<ConversationEvent>,
        ) -> Result<(), ConversationError> {
            self.calls.lock().unwrap().push(user_input.to_string());
            // Emit a minimal TurnComplete so the print task exits.
            let _ = event_tx.send(ConversationEvent::TurnComplete {
                stop_reason: crate::ports::StopReason::EndTurn,
            });
            Ok(())
        }

        async fn reset_context(
            &self,
            state: &mut ConversationState,
            _new_system_prompt: Option<String>,
        ) -> Result<ConversationCheckpoint, ConversationError> {
            Ok(ConversationCheckpoint {
                conversation_id: state.conversation_id.clone(),
                turn_count: 0,
                summary: String::new(),
                total_input_tokens: 0,
                total_output_tokens: 0,
            })
        }
    }

    /// One-shot mode: `run(Some(prompt))` must call `process_message` exactly once
    /// with the exact prompt string and then return Ok.
    #[tokio::test]
    async fn oneshot_prompt_calls_process_message_once() {
        let (mock, calls) = RecordingConversation::new();
        let adapter = CliAdapter::new(Box::new(mock));

        let result = adapter.run(Some("do the thing".to_string())).await;

        assert!(result.is_ok(), "run should succeed: {:?}", result);
        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1, "exactly one message should be sent");
        assert_eq!(recorded[0], "do the thing");
    }

    /// One-shot mode: empty prompt string must still call process_message once.
    #[tokio::test]
    async fn oneshot_empty_prompt_still_calls_process_message() {
        let (mock, calls) = RecordingConversation::new();
        let adapter = CliAdapter::new(Box::new(mock));

        let result = adapter.run(Some(String::new())).await;

        assert!(result.is_ok());
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    /// One-shot mode: multiline prompts (as spawned by workplan executor) must be
    /// sent as a single message, not split line by line.
    #[tokio::test]
    async fn oneshot_multiline_prompt_sent_as_single_message() {
        let multiline = "line one\nline two\nline three".to_string();
        let (mock, calls) = RecordingConversation::new();
        let adapter = CliAdapter::new(Box::new(mock));

        adapter.run(Some(multiline.clone())).await.unwrap();

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1, "multiline prompt must be one message, not {} messages", recorded.len());
        assert_eq!(recorded[0], multiline);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
