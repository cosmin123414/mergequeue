//! Agent backend tag. The concrete impl lives in `crate::agents`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// The coding-agent backend used to resolve conflicts. MergeQueue
/// currently ships a single backend (opencode); the enum is kept so the
/// per-repo `agent_backend` column and the `MergeAgent` seam stay stable
/// if more backends are added later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentBackend {
    Opencode,
}

impl AgentBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Opencode => "opencode",
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
            other => Err(Error::invalid(format!("unknown agent backend: {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_opencode() {
        let b = AgentBackend::Opencode;
        let parsed: AgentBackend = b.to_string().parse().unwrap();
        assert_eq!(b, parsed);
    }

    #[test]
    fn rejects_unknown() {
        assert!("nope".parse::<AgentBackend>().is_err());
    }
}
