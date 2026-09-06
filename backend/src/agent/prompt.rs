use sha2::{Digest, Sha256};

pub const AGENT_PROMPT_VERSION: &str = "agent-system/v49";
const AGENT_SYSTEM_PROMPT: &str = include_str!("../../prompts/agent_system_v49.md");

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
        assert_eq!(prompt.version, "agent-system/v49");
        assert_eq!(prompt.sha256.len(), 64);
        assert!(prompt.instructions.contains("自主选择"));
        assert!(prompt.instructions.contains("可修改的工作状态"));
        assert!(prompt.instructions.contains("ask_user_question"));
        assert!(prompt.instructions.contains("术语卡"));
        assert!(prompt.instructions.contains("AgentReportContentV1"));
        assert!(prompt.instructions.contains("先给判断"));
        assert!(prompt.instructions.contains("[[盾回前的绝刀|op:24]]"));
        assert!(prompt.instructions.contains("inspect_timeline_events"));
        assert!(prompt.instructions.contains("|ev:"));
        assert!(prompt.instructions.contains("用户最新一句话"));
        assert!(prompt.instructions.contains("用平实中文回答"));
        assert!(prompt.instructions.contains("mechanics_context"));
        assert!(prompt.instructions.contains("由你根据分析价值自主安排"));
        assert!(prompt.instructions.contains("把它更新为原任务的目标或约束"));
        assert!(prompt.instructions.len() < 12_000);
    }
}
