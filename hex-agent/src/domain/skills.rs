// Re-export shared skill types from hex-core
pub use hex_core::domain::skills::{Skill, SkillManifest, SkillTrigger};

/// Extension trait adding regex-based input matching to SkillManifest.
/// This lives in hex-agent because hex-core avoids the `regex` dependency.
pub trait SkillManifestExt {
    fn match_input(&self, input: &str) -> Vec<&Skill>;
}

impl SkillManifestExt for SkillManifest {
    fn match_input(&self, input: &str) -> Vec<&Skill> {
        let input_lower = input.to_lowercase();
        self.skills
            .iter()
            .filter(|skill| {
                skill.triggers.iter().any(|trigger| match trigger {
                    SkillTrigger::SlashCommand(cmd) => input.starts_with(cmd.as_str()),
                    SkillTrigger::Pattern(pattern) => regex::Regex::new(pattern)
                        .map(|re| re.is_match(input))
                        .unwrap_or(false),
                    SkillTrigger::Keyword(kw) => input_lower.contains(&kw.to_lowercase()),
                })
            })
            .collect()
    }
}
