//! BER to DER, as far as CMS from real mail needs it. Many writers
//! (OpenSSL, NSS) stream S/MIME as BER: indefinite lengths and octet
//! strings cut into pieces. The DER reader refuses both, so they are put
//! back together here: lengths made definite and minimal, constructed
//! `OCTET STRING`s joined, and the encrypted content of an
//! `EncryptedContentInfo` (`[0] IMPLICIT OCTET STRING`, sent in pieces)
//! joined as well. The input is hostile: depth, count and size are capped
//! and nothing panics.

/// Deepest nesting followed.
const MAX_DEPTH: usize = 48;
/// Most elements read.
const MAX_NODES: usize = 200_000;

struct Node {
    /// The identifier octets.
    tag: Vec<u8>,
    body: Body,
}

enum Body {
    Primitive(Vec<u8>),
    Constructed(Vec<Node>),
}

struct Reader<'a> {
    input: &'a [u8],
    nodes: usize,
}

const OCTET_STRING: u8 = 0x04;
const OCTET_STRING_CONSTRUCTED: u8 = 0x24;
const OID: u8 = 0x06;
const SEQUENCE: u8 = 0x30;
const CONTEXT_0_CONSTRUCTED: u8 = 0xA0;
const CONTEXT_0_PRIMITIVE: u8 = 0x80;

impl Reader<'_> {
    fn byte(&self, at: usize) -> Option<u8> {
        self.input.get(at).copied()
    }

    /// One element at `at`: it and where the next one starts.
    fn element(&mut self, at: usize, depth: usize) -> Option<(Node, usize)> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.nodes = self.nodes.checked_add(1)?;
        if self.nodes > MAX_NODES {
            return None;
        }
        let first = self.byte(at)?;
        let mut tag = vec![first];
        let mut pos = at.checked_add(1)?;
        if first & 0x1F == 0x1F {
            loop {
                let b = self.byte(pos)?;
                tag.push(b);
                pos = pos.checked_add(1)?;
                if b & 0x80 == 0 {
                    break;
                }
                if tag.len() > 6 {
                    return None;
                }
            }
        }
        let constructed = first & 0x20 != 0;
        let len_byte = self.byte(pos)?;
        pos = pos.checked_add(1)?;
        if len_byte == 0x80 {
            // Indefinite: elements until end-of-contents.
            if !constructed {
                return None;
            }
            let mut children = Vec::new();
            loop {
                if self.byte(pos)? == 0 && self.byte(pos.checked_add(1)?)? == 0 {
                    pos = pos.checked_add(2)?;
                    break;
                }
                let (child, next) = self.element(pos, depth.checked_add(1)?)?;
                children.push(child);
                pos = next;
            }
            return Some((
                Node {
                    tag,
                    body: Body::Constructed(children),
                },
                pos,
            ));
        }
        let len = if len_byte & 0x80 == 0 {
            usize::from(len_byte)
        } else {
            let count = usize::from(len_byte & 0x7F);
            if count == 0 || count > 4 {
                return None;
            }
            let mut len = 0usize;
            for _ in 0..count {
                len = len
                    .checked_mul(256)?
                    .checked_add(usize::from(self.byte(pos)?))?;
                pos = pos.checked_add(1)?;
            }
            len
        };
        let end = pos.checked_add(len)?;
        if end > self.input.len() {
            return None;
        }
        if !constructed {
            return Some((
                Node {
                    tag,
                    body: Body::Primitive(self.input.get(pos..end)?.to_vec()),
                },
                end,
            ));
        }
        let mut children = Vec::new();
        while pos < end {
            let (child, next) = self.element(pos, depth.checked_add(1)?)?;
            if next > end {
                return None;
            }
            children.push(child);
            pos = next;
        }
        Some((
            Node {
                tag,
                body: Body::Constructed(children),
            },
            end,
        ))
    }
}

/// The bytes of an octet string made of pieces, or `None` if a piece isn't
/// an octet string.
fn joined(children: &[Node]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for child in children {
        match (child.tag.as_slice(), &child.body) {
            ([OCTET_STRING], Body::Primitive(bytes)) => out.extend_from_slice(bytes),
            ([OCTET_STRING_CONSTRUCTED], Body::Constructed(inner)) => {
                out.extend(joined(inner)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

fn tag_is(node: &Node, tag: u8) -> bool {
    node.tag.as_slice() == [tag]
}

/// Put pieces together where DER wants one.
fn tidy(node: Node) -> Node {
    match node.body {
        Body::Primitive(_) => node,
        Body::Constructed(children) => {
            if node.tag.as_slice() == [OCTET_STRING_CONSTRUCTED]
                && let Some(bytes) = joined(&children)
            {
                return Node {
                    tag: vec![OCTET_STRING],
                    body: Body::Primitive(bytes),
                };
            }
            let mut children: Vec<Node> = children.into_iter().map(tidy).collect();
            // EncryptedContentInfo: SEQUENCE { OID, AlgorithmIdentifier,
            // [0] IMPLICIT OCTET STRING } with the last in pieces.
            if node.tag.as_slice() == [SEQUENCE]
                && let [first, second, third, ..] = children.as_mut_slice()
                && tag_is(first, OID)
                && tag_is(second, SEQUENCE)
                && tag_is(third, CONTEXT_0_CONSTRUCTED)
                && let Body::Constructed(pieces) = &third.body
                && let Some(bytes) = joined(pieces)
            {
                *third = Node {
                    tag: vec![CONTEXT_0_PRIMITIVE],
                    body: Body::Primitive(bytes),
                };
            }
            Node {
                tag: node.tag,
                body: Body::Constructed(children),
            }
        }
    }
}

fn write_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(u8::try_from(len).unwrap_or(0));
        return;
    }
    let bytes = len.to_be_bytes();
    let skip = bytes.iter().take_while(|b| **b == 0).count();
    let significant = bytes.get(skip..).unwrap_or_default();
    out.push(0x80 | u8::try_from(significant.len()).unwrap_or(0));
    out.extend_from_slice(significant);
}

fn write(node: &Node, out: &mut Vec<u8>) {
    out.extend_from_slice(&node.tag);
    match &node.body {
        Body::Primitive(bytes) => {
            write_len(out, bytes.len());
            out.extend_from_slice(bytes);
        }
        Body::Constructed(children) => {
            let mut inner = Vec::new();
            for child in children {
                write(child, &mut inner);
            }
            write_len(out, inner.len());
            out.extend_from_slice(&inner);
        }
    }
}

/// The DER of the one element at the start of `input` (what follows it is
/// ignored), or `None` when it isn't readable BER.
#[must_use]
pub fn to_der(input: &[u8]) -> Option<Vec<u8>> {
    let mut reader = Reader { input, nodes: 0 };
    let (node, _) = reader.element(0, 0)?;
    let mut out = Vec::with_capacity(input.len());
    write(&tidy(node), &mut out);
    Some(out)
}

/// One DER element, cut out of the bytes it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Element<'a> {
    /// Its first identifier octet.
    pub tag: u8,
    /// The whole element.
    pub whole: &'a [u8],
    /// Its content.
    pub content: &'a [u8],
    /// What follows it.
    pub rest: &'a [u8],
}

/// The DER element at the start of `der`.
#[must_use]
pub fn element(der: &[u8]) -> Option<Element<'_>> {
    let tag = *der.first()?;
    if tag & 0x1F == 0x1F {
        // High tag numbers aren't used where this is called.
        return None;
    }
    let len_byte = *der.get(1)?;
    let (len, header) = if len_byte & 0x80 == 0 {
        (usize::from(len_byte), 2usize)
    } else {
        let count = usize::from(len_byte & 0x7F);
        if count == 0 || count > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..count {
            len = len
                .checked_mul(256)?
                .checked_add(usize::from(*der.get(i.checked_add(2)?)?))?;
        }
        (len, count.checked_add(2)?)
    };
    let end = header.checked_add(len)?;
    Some(Element {
        tag,
        whole: der.get(..end)?,
        content: der.get(header..end)?,
        rest: der.get(end..)?,
    })
}

/// The elements inside a constructed DER element, each whole.
#[must_use]
pub fn children(der: &[u8]) -> Option<Vec<&[u8]>> {
    let mut content = element(der)?.content;
    let mut out = Vec::new();
    while !content.is_empty() {
        let next = element(content)?;
        out.push(next.whole);
        content = next.rest;
        if out.len() > MAX_NODES {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn walks_der() {
        let der = [0x30, 0x06, 0x02, 0x01, 0x05, 0xA0, 0x01, 0x07, 0xFF];
        let e = element(&der).unwrap();
        assert_eq!(e.tag, 0x30);
        assert_eq!(e.whole.len(), 8);
        assert_eq!(e.content, &der[2..8]);
        assert_eq!(e.rest, [0xFF]);
        assert_eq!(
            children(&der).unwrap(),
            vec![&[0x02u8, 0x01, 0x05][..], &[0xA0, 0x01, 0x07][..]]
        );
        assert!(element(&[0x30, 0x05, 0x01]).is_none());
    }

    #[test]
    fn makes_lengths_definite_and_joins_octet_strings() {
        // SEQUENCE (indefinite) { OCTET STRING in two pieces }
        let ber = [
            0x30, 0x80, 0x24, 0x80, 0x04, 0x02, b'a', b'b', 0x04, 0x01, b'c', 0x00, 0x00, 0x00,
            0x00,
        ];
        assert_eq!(
            to_der(&ber).unwrap(),
            [0x30, 0x05, 0x04, 0x03, b'a', b'b', b'c']
        );
    }

    #[test]
    fn joins_encrypted_content_sent_in_pieces() {
        // SEQUENCE { OID 1.2, SEQUENCE {}, [0] { OCTET STRING "x", OCTET STRING "y" } }
        let ber = [
            0x30, 0x80, 0x06, 0x01, 0x2A, 0x30, 0x00, 0xA0, 0x80, 0x04, 0x01, b'x', 0x04, 0x01,
            b'y', 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(
            to_der(&ber).unwrap(),
            [
                0x30, 0x09, 0x06, 0x01, 0x2A, 0x30, 0x00, 0x80, 0x02, b'x', b'y'
            ]
        );
        // An explicit [0] holding one octet string is left alone.
        let explicit = [0x30, 0x08, 0x06, 0x01, 0x2A, 0xA0, 0x03, 0x04, 0x01, b'z'];
        assert_eq!(to_der(&explicit).unwrap(), explicit);
    }

    #[test]
    fn long_lengths_and_bad_input() {
        let mut der = vec![0x04, 0x82, 0x01, 0x00];
        der.extend([7u8; 256]);
        assert_eq!(to_der(&der).unwrap(), der);
        assert!(to_der(&[0x30, 0x05, 0x04]).is_none());
        assert!(to_der(&[0x04, 0x80, 0x00, 0x00]).is_none());
        assert!(to_der(&[]).is_none());
        // Deep nesting is refused, not followed.
        let deep: Vec<u8> = std::iter::repeat_n([0x30u8, 0x80], 100).flatten().collect();
        assert!(to_der(&deep).is_none());
    }
}
