use sha2::{Digest, Sha256};

pub const AGENT_PROMPT_VERSION_V1: &str = "agent-system/v1";
const AGENT_SYSTEM_PROMPT_V1: &str = include_str!("../../prompts/agent_system_v1.md");
pub const AGENT_PROMPT_VERSION_V2: &str = "agent-system/v2";
const AGENT_SYSTEM_PROMPT_V2: &str = include_str!("../../prompts/agent_system_v2.md");
pub const AGENT_PROMPT_VERSION_V3: &str = "agent-system/v3";
const AGENT_SYSTEM_PROMPT_V3: &str = include_str!("../../prompts/agent_system_v3.md");
pub const AGENT_PROMPT_VERSION_V4: &str = "agent-system/v4";
const AGENT_SYSTEM_PROMPT_V4: &str = include_str!("../../prompts/agent_system_v4.md");
pub const AGENT_PROMPT_VERSION_V5: &str = "agent-system/v5";
const AGENT_SYSTEM_PROMPT_V5: &str = include_str!("../../prompts/agent_system_v5.md");
pub const AGENT_PROMPT_VERSION_V6: &str = "agent-system/v6";
const AGENT_SYSTEM_PROMPT_V6: &str = include_str!("../../prompts/agent_system_v6.md");
pub const AGENT_PROMPT_VERSION_V7: &str = "agent-system/v7";
const AGENT_SYSTEM_PROMPT_V7: &str = include_str!("../../prompts/agent_system_v7.md");
pub const AGENT_PROMPT_VERSION_V8: &str = "agent-system/v8";
const AGENT_SYSTEM_PROMPT_V8: &str = include_str!("../../prompts/agent_system_v8.md");
pub const AGENT_PROMPT_VERSION_V9: &str = "agent-system/v9";
const AGENT_SYSTEM_PROMPT_V9: &str = include_str!("../../prompts/agent_system_v9.md");
pub const AGENT_PROMPT_VERSION_V10: &str = "agent-system/v10";
const AGENT_SYSTEM_PROMPT_V10: &str = include_str!("../../prompts/agent_system_v10.md");
pub const AGENT_PROMPT_VERSION_V11: &str = "agent-system/v11";
const AGENT_SYSTEM_PROMPT_V11: &str = include_str!("../../prompts/agent_system_v11.md");
pub const AGENT_PROMPT_VERSION_V12: &str = "agent-system/v12";
const AGENT_SYSTEM_PROMPT_V12: &str = concat!(
    include_str!("../../prompts/agent_system_v11.md"),
    include_str!("../../prompts/agent_system_v12_addendum.md")
);
pub const AGENT_PROMPT_VERSION_V13: &str = "agent-system/v13";
const AGENT_SYSTEM_PROMPT_V13: &str = concat!(
    include_str!("../../prompts/agent_system_v11.md"),
    include_str!("../../prompts/agent_system_v12_addendum.md"),
    include_str!("../../prompts/agent_system_v13_addendum.md")
);
pub const AGENT_PROMPT_VERSION_V14: &str = "agent-system/v14";
const AGENT_SYSTEM_PROMPT_V14: &str = concat!(
    include_str!("../../prompts/agent_system_v11.md"),
    include_str!("../../prompts/agent_system_v12_addendum.md"),
    include_str!("../../prompts/agent_system_v13_addendum.md"),
    include_str!("../../prompts/agent_system_v14_addendum.md")
);

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

pub fn agent_prompt_v5() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V5.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V5,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V5,
    }
}

pub fn agent_prompt_v6() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V6.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V6,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V6,
    }
}

pub fn agent_prompt_v7() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V7.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V7,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V7,
    }
}

pub fn agent_prompt_v8() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V8.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V8,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V8,
    }
}

pub fn agent_prompt_v9() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V9.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V9,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V9,
    }
}

pub fn agent_prompt_v10() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V10.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V10,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V10,
    }
}

pub fn agent_prompt_v11() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V11.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V11,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V11,
    }
}

pub fn agent_prompt_v12() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V12.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V12,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V12,
    }
}

pub fn agent_prompt_v13() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V13.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V13,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V13,
    }
}

pub fn agent_prompt_v14() -> PromptSpec {
    let sha256 = format!("{:x}", Sha256::digest(AGENT_SYSTEM_PROMPT_V14.as_bytes()));
    PromptSpec {
        version: AGENT_PROMPT_VERSION_V14,
        sha256,
        instructions: AGENT_SYSTEM_PROMPT_V14,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_versioned_and_content_addressed() {
        let prompt = agent_prompt_v14();
        assert_eq!(prompt.version, "agent-system/v14");
        assert_eq!(prompt.sha256.len(), 64);
        assert!(prompt.instructions.contains("get_current_scenario"));
        assert!(prompt.instructions.contains("already called"));
        assert!(prompt.instructions.contains("JSON Pointer"));
        assert!(prompt.instructions.contains("Arabic numerals"));
        assert!(prompt.instructions.contains("exactly one domain"));
        assert!(prompt.instructions.contains("machine unit identifiers"));
        assert!(prompt.instructions.contains("search_knowledge_base"));
        assert!(prompt.instructions.contains("fact_eligible"));
        assert!(prompt.instructions.contains("version_match"));
        assert!(prompt.instructions.contains("category=null"));
        assert!(prompt.instructions.contains("exact season text"));
        assert!(prompt.instructions.contains("one at a time"));
        assert!(prompt.instructions.contains("coalesce redundant requests"));
        assert!(prompt
            .instructions
            .contains("only the distinctive name or nickname"));
        assert!(prompt.instructions.contains("one retrieved document"));
        assert!(prompt.instructions.contains("reference_entities"));
        assert!(prompt.instructions.contains("single point lookup"));
        assert!(prompt.instructions.contains("分山劲·悟"));
        assert!(prompt.instructions.contains("计算器未实现无界端"));
        assert!(prompt.instructions.contains("rotation_input.mode"));
        assert!(prompt.instructions.contains("rotation_changes"));
        assert!(prompt.instructions.contains("flagship client"));
        assert!(prompt
            .instructions
            .contains("Unless the user explicitly says otherwise"));
        assert!(prompt.instructions.contains("selected adaptively"));
        assert!(prompt.instructions.contains("selection` summary"));
        assert!(prompt
            .instructions
            .contains("exact value appears in the cited result"));
        assert!(prompt
            .instructions
            .contains("Never borrow a number from an uncited result"));
        assert!(prompt.instructions.contains("Observation"));
        assert!(prompt.instructions.contains("结论 → 主要瓶颈"));
        assert!(prompt.instructions.contains("typed candidate field"));
        assert!(prompt.instructions.contains("hard response budget"));
        assert!(prompt.instructions.contains("1 to 3 findings"));
        assert!(prompt.instructions.contains("analysis_plan"));
        assert!(prompt.instructions.contains("evidence_pack"));
        assert!(prompt.instructions.contains("domain_claims"));
        assert!(prompt
            .instructions
            .contains("orange_weapon_dot_not_implemented"));
        assert!(prompt.instructions.contains("high-quality output"));
    }
}
