//! A small element tree for Exchange's XML answers, and escaping for the
//! requests. Answers are read as untrusted: a size cap, a depth cap and an
//! element cap, and names are matched by local name (no namespace
//! prefixes), as servers vary in the prefixes they write.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::SyncError;
use crate::dav::unescape_xml;

/// Deepest nesting read.
const MAX_DEPTH: usize = 64;
/// Most elements read from one answer.
const MAX_ELEMENTS: usize = 500_000;

/// One element: its local name, attributes (local names), text and
/// children.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Element {
    /// Local name (`Folder` for `t:Folder`).
    pub name: String,
    /// Attributes by local name, values unescaped.
    pub attrs: Vec<(String, String)>,
    /// The text directly inside it (all text pieces joined).
    pub text: String,
    /// Child elements, in order.
    pub children: Vec<Element>,
}

fn local_name(name: &str) -> String {
    name.rsplit(':').next().unwrap_or_default().to_owned()
}

fn start(e: &BytesStart<'_>) -> Element {
    let attrs = e
        .attributes()
        .flatten()
        .map(|a| (local_name(a.key.as_ref()), unescape_xml(&a.value)))
        .collect();
    Element {
        name: local_name(e.name().as_ref()),
        attrs,
        text: String::new(),
        children: Vec::new(),
    }
}

impl Element {
    /// Read a whole document; its root element.
    ///
    /// # Errors
    /// [`SyncError::Protocol`] for XML that can't be read or is too big.
    pub fn parse(xml: &str) -> Result<Self, SyncError> {
        let bad =
            |what: String| SyncError::Protocol(format!("unreadable reply from Exchange: {what}"));
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);
        let mut stack: Vec<Element> = Vec::new();
        let mut root: Option<Element> = None;
        let mut count = 0usize;
        loop {
            let event = reader.read_event().map_err(|e| bad(e.to_string()))?;
            match event {
                Event::Start(e) => {
                    count = count.saturating_add(1);
                    if stack.len() >= MAX_DEPTH || count > MAX_ELEMENTS {
                        return Err(bad("nested too deep or too large".into()));
                    }
                    stack.push(start(&e));
                }
                Event::Empty(e) => {
                    count = count.saturating_add(1);
                    if count > MAX_ELEMENTS {
                        return Err(bad("too large".into()));
                    }
                    let element = start(&e);
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(element),
                        None => root = Some(element),
                    }
                }
                Event::Text(t) => {
                    if let Some(current) = stack.last_mut() {
                        current.text.push_str(&unescape_xml(t.as_ref()));
                    }
                }
                Event::GeneralRef(r) => {
                    if let Some(current) = stack.last_mut() {
                        let name: &str = r.as_ref();
                        current.text.push_str(&unescape_xml(&format!("&{name};")));
                    }
                }
                Event::CData(c) => {
                    if let Some(current) = stack.last_mut() {
                        let data: &str = c.as_ref();
                        current.text.push_str(data);
                    }
                }
                Event::End(_) => {
                    let Some(done) = stack.pop() else {
                        return Err(bad("an end with no start".into()));
                    };
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(done),
                        None => root = Some(done),
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        root.ok_or_else(|| bad("no document".into()))
    }

    /// The first child with this local name.
    #[must_use]
    pub fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every child with this local name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Self> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The element at this path of child names.
    #[must_use]
    pub fn at(&self, path: &[&str]) -> Option<&Self> {
        path.iter().try_fold(self, |at, name| at.child(name))
    }

    /// Every element with this local name, anywhere below, in document
    /// order.
    #[must_use]
    pub fn descendants(&self, name: &str) -> Vec<&Self> {
        let mut out = Vec::new();
        let mut stack: Vec<&Self> = self.children.iter().rev().collect();
        while let Some(next) = stack.pop() {
            if next.name == name {
                out.push(next);
            }
            stack.extend(next.children.iter().rev());
        }
        out
    }

    /// The first element with this local name anywhere below.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&Self> {
        let mut stack: Vec<&Self> = self.children.iter().rev().collect();
        while let Some(next) = stack.pop() {
            if next.name == name {
                return Some(next);
            }
            stack.extend(next.children.iter().rev());
        }
        None
    }

    /// An attribute's value.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// A child's text, trimmed; `None` when there is no such child or it is
    /// empty.
    #[must_use]
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name)
            .map(|c| c.text.trim())
            .filter(|t| !t.is_empty())
    }
}

/// Escape text for an XML element or a double-quoted attribute. Control
/// characters XML can't carry are dropped.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}
