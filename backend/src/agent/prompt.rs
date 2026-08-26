use sha2::{Digest, Sha256};

pub const AGENT_PROMPT_VERSION_V1: &str = "agent-system/v1";
const AGENT_SYSTEM_PROMPT_V1: &str = include_str!("../../prompts/agent_system_v1.md");
pub const AGENT_PROMPT_VERSION_V2: &str = "agent-system/v2";
const AGENT_SYSTEM_PROMPT_V2: &str = include_str!("../../prompts/agent_system_v2.md");
pub const AGENT_PROMPT_VERSION_V3: &str = "agent-system/v3";
const AGENT_SYSTEM_PROMPT_V3: &str = include_str!("../../prompts/agent_system_v3.md");
pub const AGENT_PROMPT_VERSION_V4: &str = "agent-system/v4";
const AGENT_SYSTEM_PROMPT_V4: &str = include_str!("../../prompts/agent_system_v4.md");

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

pub fn agent_prompt_v2() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V2.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V2,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V2,
    }
}

pub fn agent_prompt_v3() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V3.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V3,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V3,
    }
}

pub fn agent_prompt_v4() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V4.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V4,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_versioned_and_content_addressed() {
        let prompt = agent_prompt_v4();
        assert_eq!(prompt.version, "agent-system/v4");
        assert_eq!(prompt.sha256.len(), 64);
        assert!(prompt.instructions.contains("get_current_scenario"));
        assert!(prompt.instructions.contains("already called"));
        assert!(prompt.instructions.contains("JSON Pointer"));
        assert!(prompt.instructions.contains("Arabic numerals"));
        assert!(prompt
            .instructions
            .contains("exactly one domain experiment"));
        assert!(prompt.instructions.contains("machine unit identifiers"));
    }
}
