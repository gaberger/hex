use serde::{Deserialize, Serialize};

/// A skill loaded from a markdown file with YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub triggers: Vec<SkillTrigger>,
    pub body: String,
    pub source_path: String,
}

/// How a skill gets triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillTrigger {
    SlashCommand(String),
    Pattern(String),
    Keyword(String),
}

/// Summary of all loaded skills — used for system prompt injection.
#[derive(Debug, Clone, Default)]
pub struct SkillManifest {
    pub skills: Vec<Skill>,
}

impl SkillManifest {
    /// Generate a summary for the system prompt — lists available skills.
    pub fn system_prompt_section(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut out = String::from("# Available Skills\n\n");
        for skill in &self.skills {
            let triggers: Vec<String> = skill
                .triggers
                .iter()
                .map(|t| match t {
                    SkillTrigger::SlashCommand(cmd) => cmd.clone(),
                    SkillTrigger::Pattern(p) => format!("/{}", p),
                    SkillTrigger::Keyword(kw) => kw.clone(),
                })
                .collect();
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
