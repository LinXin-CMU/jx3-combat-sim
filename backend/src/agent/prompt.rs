use sha2::{Digest, Sha256};

pub const AGENT_PROMPT_VERSION: &str = "agent-system/v28";
const AGENT_SYSTEM_PROMPT: &str = include_str!("../../prompts/agent_system_v28.md");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSpec {
    pub version: &'static str,
    pub sha256: String,
    pub instructions: &'static str,
}

pub fn agent_prompt() -> PromptSpec {
    PromptSpec {
        version: AGENT_PROMPT_VERSION,
        sha256: format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT.as_bytes())),
        instructions: AGENT_SYSTEM_PROMPT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_prompt_is_single_model_led_instruction() {
        let prompt = agent_prompt();
        assert_eq!(prompt.version, "agent-system/v28");
        assert_eq!(prompt.sha256.len(), 64);
        assert!(prompt.instructions.contains("自主选择"));
        assert!(prompt.instructions.contains("可修改的工作计划"));
        assert!(prompt.instructions.contains("ask_user_question"));
        assert!(prompt.instructions.contains("AgentReportContentV1"));
        assert!(prompt.instructions.len() < 6_000);
    }
}
