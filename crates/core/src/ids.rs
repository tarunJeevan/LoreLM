use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[doc = concat!("Stable identifier for ", stringify!($name), ".")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a new random ID.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Creates an ID from an existing UUID.
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Returns the wrapped UUID.
            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(input: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(input)?))
            }
        }
    };
}

id_type!(WorkspaceId);
id_type!(DocumentId);
id_type!(ConversationId);
id_type!(MessageId);
id_type!(ModelId);
id_type!(ChunkId);

/// Stable identifier for a mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModeId(String);

impl ModeId {
    /// Creates a mode ID from a static or configured name.
    pub fn named(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModeId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
