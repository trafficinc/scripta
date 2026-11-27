use std::{borrow::Borrow, fmt, ops::Deref, rc::Rc};

/// Immutable UTF-8 map-key text. Clones share text; replacing a key never
/// changes the text of another collection snapshot.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapKeyText(Rc<str>);

impl MapKeyText {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Copy key text into an ordinary owned script/host string.
    pub fn into_string(self) -> String {
        self.as_str().to_owned()
    }
}

impl From<&str> for MapKeyText {
    fn from(text: &str) -> Self {
        Self(Rc::from(text))
    }
}
impl From<String> for MapKeyText {
    fn from(text: String) -> Self {
        Self(Rc::from(text))
    }
}
impl From<Rc<str>> for MapKeyText {
    fn from(text: Rc<str>) -> Self {
        Self(text)
    }
}
impl From<MapKeyText> for String {
    fn from(text: MapKeyText) -> Self {
        text.into_string()
    }
}
impl Deref for MapKeyText {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<str> for MapKeyText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl Borrow<str> for MapKeyText {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
impl fmt::Display for MapKeyText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}
impl fmt::Debug for MapKeyText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}
impl PartialEq<str> for MapKeyText {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<&str> for MapKeyText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialEq<String> for MapKeyText {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<MapKeyText> for str {
    fn eq(&self, other: &MapKeyText) -> bool {
        self == other.as_str()
    }
}
impl PartialEq<MapKeyText> for String {
    fn eq(&self, other: &MapKeyText) -> bool {
        self == other.as_str()
    }
}

#[cfg(test)]
#[path = "map_key_text_tests.rs"]
mod tests;
