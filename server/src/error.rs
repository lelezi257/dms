//! Errors raised while constructing shared domain values.

use std::fmt;

/// Error returned when a process component name is unknown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseComponentKindError(pub(crate) String);

impl fmt::Display for ParseComponentKindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown DMS component: {}", self.0)
    }
}

impl std::error::Error for ParseComponentKindError {}

/// Error returned when a process identity is not safe for shared contracts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidNodeId(pub(crate) String);

impl fmt::Display for InvalidNodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid DMS node id: {:?}", self.0)
    }
}

impl std::error::Error for InvalidNodeId {}
