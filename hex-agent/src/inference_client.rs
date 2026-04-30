use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum InferenceTier {
    T1, // scaffold/transform/script → qwen3:4b
    T2, // codegen → qwen2.5-coder:32b
    T2_5, // complex reasoning → devstral-small-2:24b
}

impl InferenceTier {
    pub fn from_strategy_hint(hint: Option<&str>) -> Self {
        match hint {
            Some("scaffold") | Some("transform") | Some("script") => InferenceTier::T1,
            Some("codegen") => InferenceTier::T2,
            Some("inference") => InferenceTier::T2_5,
            _ => InferenceTier::T2, // default to T2 for unknown
        }
    }

    pub fn model_name(&self) -> &str {
        match self {
            InferenceTier::T1 => "qwen3:4b",
            InferenceTier::T2 => "qwen2.5-coder:32b",
            InferenceTier::T2_5 => "devstral-small-2:24b",
        }
    }
}

#[derive(Debug, Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f32,
    num_predict: i32,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    response: String,
    thinking: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenRouterRequest {
    model: String,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct ClaudeRequest {
    model: String,
    messages: Vec<Message>,
    max_tokens: u32,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct ClaudeResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: MessageResponse,
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    text: String,
}

pub struct InferenceClient {
    base_url: String,
}

pub struct OpenRouterClient {
    api_key: String,
}

pub struct ClaudeClient {
    api_key: String,
}

impl InferenceClient {
    pub fn new() -> Self {
        let base_url = std::env::var("OLLAMA_HOST")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        Self { base_url }
    }

    pub async fn generate(&self, tier: InferenceTier, prompt: String) -> Result<String> {
        let model = tier.model_name().to_string();

        let request = OllamaRequest {
            model: model.clone(),
            prompt,
            stream: false,
            options: OllamaOptions {
                temperature: 0.2,
                num_predict: 2048,
            },
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        let response = client
            .post(format!("{}/api/generate", self.base_url))
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Failed to call Ollama with model {}", model))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama request failed ({}): {}", status, body);
        }

        let body_text = response.text().await.context("Failed to read response body")?;

        let ollama_response: OllamaResponse = serde_json::from_str(&body_text)
            .context("Failed to parse Ollama JSON response")?;

        // qwen3 and other thinking models put output in "thinking" field, not "response"
        let content = if ollama_response.response.is_empty() {
            ollama_response.thinking.unwrap_or_default()
        } else {
            ollama_response.response
        };

        Ok(content)
    }

    pub fn build_task_prompt(
        task_title: &str,
        files: &[String],
        evidence: &[String],
    ) -> String {
        let mut prompt = String::new();

        prompt.push_str("You are a code generation assistant. Your task:\n\n");
        prompt.push_str(&format!("TASK: {}\n\n", task_title));

        if !files.is_empty() {
            prompt.push_str("FILES TO CREATE/MODIFY:\n");
            for file in files {
                prompt.push_str(&format!("- {}\n", file));
            }
            prompt.push_str("\n");
        }

        if !evidence.is_empty() {
            prompt.push_str("VALIDATION COMMANDS (will be run after):\n");
            for cmd in evidence {
                prompt.push_str(&format!("- {}\n", cmd));
            }
            prompt.push_str("\n");
        }

        prompt.push_str("INSTRUCTIONS:\n");
        prompt.push_str("1. If file exists, ADD to it (append/insert) - do NOT replace entire file\n");
        prompt.push_str("2. For new files, generate complete working code\n");
        prompt.push_str("3. Use proper syntax and imports\n");
        prompt.push_str("4. Follow the task requirements exactly\n");
        prompt.push_str("5. Output ONLY the file contents, no explanations\n");
        prompt.push_str("6. Prefer small additions over complete rewrites\n");
        prompt.push_str("7. CRITICAL: Do NOT wrap output in markdown code blocks (no ```rust or ``` fences)\n");
        prompt.push_str("8. Use this format:\n\n");
        prompt.push_str("=== FILE: path/to/file.rs ===\n");
        prompt.push_str("<file contents here>\n");
        prompt.push_str("=== END FILE ===\n\n");
        prompt.push_str("Generate the code now:\n");

        prompt
    }

    pub fn parse_response(response: &str) -> Result<Vec<(String, String)>> {
        // Strip markdown code fences if LLM added them despite instructions
        let cleaned_response = response
            .trim_start_matches("```rust")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let mut files = Vec::new();
        let mut current_file = None;
        let mut current_content = String::new();

        for line in cleaned_response.lines() {
            if line.starts_with("=== FILE:") {
                // Save previous file if exists
                if let Some(path) = current_file.take() {
                    files.push((path, current_content.trim().to_string()));
                    current_content.clear();
                }
                // Extract new file path
                let path = line
                    .trim_start_matches("=== FILE:")
                    .trim_end_matches("===")
                    .trim()
                    .to_string();
                current_file = Some(path);
            } else if line.starts_with("=== END FILE ===") {
                // Save current file
                if let Some(path) = current_file.take() {
                    files.push((path, current_content.trim().to_string()));
                    current_content.clear();
                }
            } else if current_file.is_some() {
                current_content.push_str(line);
                current_content.push('\n');
            }
        }

        // Save last file if not closed with END FILE
        if let Some(path) = current_file {
            files.push((path, current_content.trim().to_string()));
        }

        if files.is_empty() {
            anyhow::bail!("No files found in LLM response. Expected format: === FILE: path === ... === END FILE ===");
        }

        Ok(files)
    }
}

impl Default for InferenceClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenRouterClient {
    pub fn new() -> Option<Self> {
        // Try env first, then hex secrets vault
        let api_key = std::env::var("OPENROUTER_API_KEY").ok()
            .or_else(|| Self::get_from_vault("OPENROUTER_API_KEY"))?;
        Some(Self { api_key })
    }

    fn get_from_vault(key: &str) -> Option<String> {
        std::process::Command::new("hex")
            .args(["secrets", "get", key])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    }

    pub async fn generate(&self, prompt: String) -> Result<String> {
        let request = OpenRouterRequest {
            model: "deepseek/deepseek-coder".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt,
            }],
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        let response = client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .context("Failed to call OpenRouter API")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenRouter request failed ({}): {}", status, body);
        }

        let api_response: OpenRouterResponse = response
            .json()
            .await
            .context("Failed to parse OpenRouter response")?;

        Ok(api_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default())
    }
}

impl ClaudeClient {
    pub fn new() -> Option<Self> {
        // Try env first, then hex secrets vault
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok()
            .or_else(|| Self::get_from_vault("ANTHROPIC_API_KEY"))?;
        Some(Self { api_key })
    }

    fn get_from_vault(key: &str) -> Option<String> {
        std::process::Command::new("hex")
            .args(["secrets", "get", key])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    }

    pub async fn generate(&self, prompt: String) -> Result<String> {
        let request = ClaudeRequest {
            model: "claude-sonnet-4".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt,
            }],
            max_tokens: 4096,
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&request)
            .send()
            .await
            .context("Failed to call Claude API")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Claude request failed ({}): {}", status, body);
        }

        let api_response: ClaudeResponse = response
            .json()
            .await
            .context("Failed to parse Claude response")?;

        Ok(api_response
            .content
            .first()
            .map(|c| c.text.clone())
            .unwrap_or_default())
    }
}
