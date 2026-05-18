//! Crate-wide error type. Adapters add their own `From` impls.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("schema version {found} is newer than the binary supports ({supported})")]
    SchemaTooNew { found: i64, supported: i64 },

    #[error("not a git repository: {0}")]
    NotAGitRepo(String),

    #[error("git: {0}")]
    Git(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("state-root not resolvable; set $MERGESMITH_HOME")]
    StateRootMissing,

    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("toml ser: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("other: {0}")]
    Other(String),
}

impl Error {
    pub fn other(s: impl Into<String>) -> Self {
        Self::Other(s.into())
    }

    pub fn invalid(s: impl Into<String>) -> Self {
        Self::Invalid(s.into())
    }
}
