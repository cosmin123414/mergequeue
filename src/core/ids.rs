//! Newtype IDs wrapping `Uuid`. The newtype layer prevents accidentally
//! passing a `RepoId` where a `QueueEntryId` is expected.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Six-character short form for human-readable display.
            pub fn short(&self) -> String {
                let s = self.0.simple().to_string();
                s[..6].to_string()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

define_id!(RepoId);
define_id!(QueueEntryId);
define_id!(ConflictSessionId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_via_string() {
        let id = RepoId::new();
        let s = id.to_string();
        let parsed: RepoId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn short_is_six_chars() {
        assert_eq!(QueueEntryId::new().short().len(), 6);
    }
}
