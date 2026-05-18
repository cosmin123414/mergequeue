//! Agent backend tag. The concrete impl lives in `crate::agents`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentBackend {
    Opencode,
    ClaudeCode,
    Cursor,
    Codex,
}

impl AgentBackend {
    pub fn all() -> [Self; 4] {
        [Self::Opencode, Self::ClaudeCode, Self::Cursor, Self::Codex]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Opencode => "opencode",
            Self::ClaudeCode => "claude_code",
            Self::Cursor => "cursor",
            Self::Codex => "codex",
        }
    }
}

impl fmt::Display for AgentBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AgentBackend {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "opencode" => Ok(Self::Opencode),
            "claude_code" | "claude-code" | "claude" => Ok(Self::ClaudeCode),
            "cursor" | "cursor-agent" => Ok(Self::Cursor),
            "codex" => Ok(Self::Codex),
            other => Err(Error::invalid(format!("unknown agent backend: {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_known_backends() {
        for b in AgentBackend::all() {
            let s = b.to_string();
            let parsed: AgentBackend = s.parse().unwrap();
            assert_eq!(b, parsed);
        }
    }

    #[test]
    fn rejects_unknown() {
        assert!("nope".parse::<AgentBackend>().is_err());
    }
}
