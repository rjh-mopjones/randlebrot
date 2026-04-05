//! Error types for artifact storage operations.

use std::fmt;
use std::path::PathBuf;

/// Errors that can occur during artifact storage operations.
#[derive(Debug)]
pub enum ArtifactError {
    /// Failed to determine the home directory.
    NoHomeDirectory,
    /// The provided tag contains invalid characters.
    InvalidTag {
        tag: String,
        reason: &'static str,
    },
    /// An artifact with this tag already exists.
    AlreadyExists {
        kind: String,
        tag: String,
    },
    /// The requested artifact was not found.
    NotFound {
        kind: String,
        tag: String,
    },
    /// A file within the artifact could not be found.
    FileNotFound {
        path: PathBuf,
    },
    /// Filesystem I/O error with context.
    Io {
        context: String,
        source: std::io::Error,
    },
    /// Bincode serialization/deserialization error.
    Bincode {
        context: String,
        source: bincode::Error,
    },
    /// RON serialization error.
    RonSerialize {
        context: String,
        source: ron::Error,
    },
    /// RON deserialization error.
    RonDeserialize {
        context: String,
        source: ron::error::SpannedError,
    },
    /// Image save error.
    Image {
        context: String,
        source: image::ImageError,
    },
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHomeDirectory => {
                write!(f, "could not determine home directory")
            }
            Self::InvalidTag { tag, reason } => {
                write!(f, "invalid artifact tag '{tag}': {reason}")
            }
            Self::AlreadyExists { kind, tag } => {
                write!(f, "{kind} artifact '{tag}' already exists")
            }
            Self::NotFound { kind, tag } => {
                write!(f, "{kind} artifact '{tag}' not found")
            }
            Self::FileNotFound { path } => {
                write!(f, "file not found: {}", path.display())
            }
            Self::Io { context, source } => {
                write!(f, "{context}: {source}")
            }
            Self::Bincode { context, source } => {
                write!(f, "{context}: {source}")
            }
            Self::RonSerialize { context, source } => {
                write!(f, "{context}: {source}")
            }
            Self::RonDeserialize { context, source } => {
                write!(f, "{context}: {source}")
            }
            Self::Image { context, source } => {
                write!(f, "{context}: {source}")
            }
        }
    }
}

impl std::error::Error for ArtifactError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Bincode { source, .. } => Some(source.as_ref()),
            Self::RonSerialize { source, .. } => Some(source),
            Self::RonDeserialize { source, .. } => Some(source),
            Self::Image { source, .. } => Some(source),
            _ => None,
        }
    }
}
