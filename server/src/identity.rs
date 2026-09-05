//! Transport-independent domain values shared by DMS components.

use std::{fmt, str::FromStr};

use crate::{InvalidNodeId, ParseComponentKindError};

/// Stable identity of a process kind exposed by the operational protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentKind {
    /// Per-compute-node data process.
    Node,
    /// Cluster metadata process.
    Meta,
}

impl ComponentKind {
    /// Returns the stable wire name of the component.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Node => "dms-node",
            Self::Meta => "dms-meta",
        }
    }
}

impl fmt::Display for ComponentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ComponentKind {
    type Err = ParseComponentKindError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "dms-node" => Ok(Self::Node),
            "dms-meta" => Ok(Self::Meta),
            _ => Err(ParseComponentKindError(value.to_owned())),
        }
    }
}

/// Validated identity of one DMS process instance.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NodeId(String);

impl NodeId {
    /// Creates an identity suitable for logs, metrics, and protocol messages.
    ///
    /// Identities are intentionally conservative: 1–63 ASCII letters, digits,
    /// dots, underscores, or hyphens. This keeps them safe in the initial text
    /// protocol while leaving the storage model independent of DNS rules.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidNodeId> {
        let value = value.into();
        let valid_length = (1..=63).contains(&value.len());
        let valid_bytes = value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if valid_length && valid_bytes {
            Ok(Self(value))
        } else {
            Err(InvalidNodeId(value))
        }
    }

    /// Returns the validated identity as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for NodeId {
    type Err = InvalidNodeId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{ComponentKind, NodeId};

    #[test]
    fn node_identity_rejects_wire_delimiters() {
        assert!(NodeId::new("n1").is_ok());
        assert!(NodeId::new("rack-a.node_01").is_ok());
        assert!(NodeId::new("").is_err());
        assert!(NodeId::new("node with spaces").is_err());
        assert!(NodeId::new("node=one").is_err());
    }

    #[test]
    fn component_wire_names_round_trip() {
        for component in [ComponentKind::Node, ComponentKind::Meta] {
            assert_eq!(component.as_str().parse(), Ok(component));
        }
    }
}
