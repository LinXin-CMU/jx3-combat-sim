use sha2::{Digest, Sha256};

pub const AGENT_PROMPT_VERSION_V1: &str = "agent-system/v1";
const AGENT_SYSTEM_PROMPT_V1: &str = include_str!("../../prompts/agent_system_v1.md");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSpec {
    pub version: &'static str,
    pub sha256: String,
    pub instructions: &'static str,
}

pub fn agent_prompt_v1() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V1.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V1,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_versioned_and_content_addressed() {
        let prompt = agent_prompt_v1();
        assert_eq!(prompt.version, "agent-system/v1");
        assert_eq!(prompt.sha256.len(), 64);
        assert!(prompt.instructions.contains("get_current_scenario"));
        assert!(prompt.instructions.contains("JSON Pointer"));
    }
}
