//! `RegisteredRepo` and per-repo CI configuration.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::core::agent_backend::AgentBackend;
use crate::core::ids::RepoId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredRepo {
    pub id: RepoId,
    pub root_path: PathBuf,
    pub default_branch: String,
    pub ci: RepoCiConfig,
    pub agent_backend: AgentBackend,
    #[serde(with = "time::serde::iso8601")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::iso8601")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoCiConfig {
    pub lint_command: Option<String>,
    pub test_command: Option<String>,
    pub build_command: Option<String>,
    #[serde(with = "duration_ms")]
    pub dirty_retry: Duration,
}

impl Default for RepoCiConfig {
    fn default() -> Self {
        Self {
            lint_command: None,
            test_command: None,
            build_command: None,
            dirty_retry: Duration::from_secs(30),
        }
    }
}

mod duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        // u64 ms fits any realistic retry; truncating overflow is fine.
        s.serialize_u64(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}
