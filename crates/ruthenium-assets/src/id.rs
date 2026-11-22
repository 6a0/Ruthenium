//! Resource identifiers (`namespace:path`).

use std::cmp::Ordering;
use std::fmt;

/// The namespace an identifier without one gets.
pub const DEFAULT_NAMESPACE: &str = "minecraft";

/// A resource location such as `minecraft:block/stone`.
///
/// Ordered by path first, then namespace, as vanilla orders them
/// (`resources/Identifier.java` `compareTo`). Sorted listings and the stitcher's
/// tie-break depend on this order.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Id {
    namespace: Box<str>,
    path: Box<str>,
}

/// Characters allowed in a namespace: `[a-z0-9_.-]`.
pub fn valid_namespace(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'))
}

/// Characters allowed in a path: `[a-z0-9_./-]`.
pub fn valid_path(s: &str) -> bool {
    s.bytes()
        .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
}

impl Id {
    /// `None` if either part has a character vanilla rejects.
    pub fn new(namespace: &str, path: &str) -> Option<Id> {
        (valid_namespace(namespace) && valid_path(path)).then(|| Id {
            namespace: namespace.into(),
            path: path.into(),
        })
    }

    /// In the default namespace.
    pub fn minecraft(path: &str) -> Option<Id> {
        Id::new(DEFAULT_NAMESPACE, path)
    }

    /// Parses `namespace:path` or a bare `path` (default namespace).
    pub fn parse(s: &str) -> Option<Id> {
        match s.split_once(':') {
            Some((ns, path)) => Id::new(if ns.is_empty() { DEFAULT_NAMESPACE } else { ns }, path),
            None => Id::minecraft(s),
        }
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The same namespace with another path. The path is not re-checked, so
    /// callers only pass paths built from valid parts.
    pub fn with_path(&self, path: impl Into<Box<str>>) -> Id {
        Id {
            namespace: self.namespace.clone(),
            path: path.into(),
        }
    }

    /// `prefix` + path + `suffix`, in the same namespace.
    pub fn wrap(&self, prefix: &str, suffix: &str) -> Id {
        self.with_path(format!("{prefix}{}{suffix}", self.path))
    }
}

impl Ord for Id {
    fn cmp(&self, other: &Self) -> Ordering {
        self.path
            .cmp(&other.path)
            .then_with(|| self.namespace.cmp(&other.namespace))
    }
}

impl PartialOrd for Id {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.path)
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates() {
        assert_eq!(Id::parse("stone").unwrap().to_string(), "minecraft:stone");
        assert_eq!(Id::parse(":stone").unwrap().to_string(), "minecraft:stone");
        assert_eq!(Id::parse("foo:a/b.png").unwrap().namespace(), "foo");
        assert!(Id::parse("Foo:a").is_none());
        assert!(Id::parse("foo:a b").is_none());
        assert!(Id::parse("foo:a:b").is_none());
        assert!(Id::new("", "a").is_none());
    }

    #[test]
    fn orders_by_path_then_namespace() {
        let mut ids: Vec<Id> = ["b:a", "a:b", "a:a"]
            .iter()
            .map(|s| Id::parse(s).unwrap())
            .collect();
        ids.sort();
        let names: Vec<String> = ids.iter().map(Id::to_string).collect();
        assert_eq!(names, ["a:a", "b:a", "a:b"]);
    }
}
