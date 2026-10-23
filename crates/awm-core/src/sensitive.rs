//! Text read from agent data, wrapped so it can't be printed by accident.
//!
//! Agent data holds conversations, code and sometimes secrets (requirements.md,
//! REQ-26). Everything the tool reads from it is held as [`AgentBytes`], which
//! implements neither `Display` nor `serde::Serialize`, and whose `Debug` output says
//! only how long it is (design.md, "Key design decisions", 11). Code that needs the
//! content, to compare it or to rewrite a path in it, asks for it explicitly with
//! [`AgentBytes::as_bytes`] or [`AgentBytes::as_str`].

use std::fmt;

/// Bytes read from agent data, editor data or process files.
///
/// It can't be formatted for display:
///
/// ```compile_fail
/// let secret = awm_core::sensitive::AgentBytes::new(b"token".to_vec());
/// println!("{secret}");
/// ```
///
/// and its `Debug` form gives only its length:
///
/// ```
/// let secret = awm_core::sensitive::AgentBytes::new(b"token".to_vec());
/// assert_eq!(format!("{secret:?}"), "<redacted, 5 bytes>");
/// ```
#[derive(Clone, PartialEq, Eq, Default)]
pub struct AgentBytes(Vec<u8>);

impl AgentBytes {
    /// Wraps bytes read from agent data.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The content, for code that compares or rewrites it. Never pass it to output.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The content as text, if it's valid UTF-8. Never pass it to output.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// How many bytes it holds.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether it's empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for AgentBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted, {} bytes>", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::AgentBytes;

    #[test]
    fn gives_access_only_when_asked() {
        let bytes = AgentBytes::new("a secret".into());
        assert_eq!(bytes.as_bytes(), b"a secret");
        assert_eq!(bytes.as_str(), Some("a secret"));
        assert_eq!(bytes.len(), 8);
        assert!(!bytes.is_empty());
        assert!(AgentBytes::default().is_empty());
        assert_eq!(AgentBytes::new(vec![0xff]).as_str(), None);
        assert!(!format!("{bytes:?}").contains("secret"));
    }
}
