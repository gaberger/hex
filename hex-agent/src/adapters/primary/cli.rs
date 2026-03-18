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
}

impl CliAdapter {
    pub fn new(conversation: Box<dyn ConversationPort>) -> Self {
        Self { conversation }
    }

    /// Run the interactive REPL.
    pub async fn run(&self) -> anyhow::Result<()> {
        let mut state = ConversationState::new(uuid::Uuid::new_v4().to_string());

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
                    // Leak is safe here — the string lives for the loop iteration
                    // and the borrow checker needs a &str, not String
                    Box::leak(format!(
                        "Create a hex workplan for: {}", plan_args
                    ).into_boxed_str()) as &str
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
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
