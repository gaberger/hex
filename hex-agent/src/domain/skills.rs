use serde::{Deserialize, Serialize};

/// A skill loaded from a markdown file with YAML frontmatter.
///
/// Skills are prompt templates that get injected into the conversation
/// when triggered by user input patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Unique skill identifier (e.g., "hex-scaffold")
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Patterns that trigger this skill
    pub triggers: Vec<SkillTrigger>,
    /// The full prompt body (markdown content after frontmatter)
    pub body: String,
    /// File path the skill was loaded from
    pub source_path: String,
}

/// How a skill gets triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillTrigger {
    /// Exact slash command (e.g., "/hex-scaffold")
    SlashCommand(String),
    /// Regex pattern match on user input
    Pattern(String),
    /// Keyword match (case-insensitive)
    Keyword(String),
}

/// Summary of all loaded skills — used for system prompt injection.
#[derive(Debug, Clone, Default)]
pub struct SkillManifest {
    pub skills: Vec<Skill>,
}

impl SkillManifest {
    /// Find skills triggered by a user message.
    pub fn match_input(&self, input: &str) -> Vec<&Skill> {
        let input_lower = input.to_lowercase();
        self.skills
            .iter()
            .filter(|skill| {
                skill.triggers.iter().any(|trigger| match trigger {
                    SkillTrigger::SlashCommand(cmd) => input.starts_with(cmd.as_str()),
                    SkillTrigger::Pattern(pattern) => {
                        regex::Regex::new(pattern)
                            .map(|re| re.is_match(input))
                            .unwrap_or(false)
                    }
                    SkillTrigger::Keyword(kw) => input_lower.contains(&kw.to_lowercase()),
                })
            })
            .collect()
    }

    /// Generate a summary for the system prompt — lists available skills.
    pub fn system_prompt_section(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut out = String::from("# Available Skills\n\n");
        for skill in &self.skills {
            let triggers: Vec<String> = skill.triggers.iter().map(|t| match t {
                SkillTrigger::SlashCommand(cmd) => cmd.clone(),
                SkillTrigger::Pattern(p) => format!("/{}", p),
                SkillTrigger::Keyword(kw) => kw.clone(),
            }).collect();
            out.push_str(&format!(
                "- **{}**: {} (triggers: {})\n",
                skill.name,
                skill.description,
                triggers.join(", ")
            ));
        }
        out
    }
}
