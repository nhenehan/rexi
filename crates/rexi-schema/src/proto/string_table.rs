//! The starting contents of the string table for a set of schemas
//! ([EXI Appendix D](https://www.w3.org/TR/exi/#initialStringValues),
//! [7.3.1](https://www.w3.org/TR/exi/#stringTablePartitions)).
//!
//! The string table holds strings already seen, so a repeat can be sent as a short
//! number (a "compact identifier") instead of the full text. It is split into
//! partitions. There is one URI partition, and for each URI a partition of
//! prefixes and a partition of local names. With a schema, these partitions start
//! out filled with the names the schema declares.

use std::collections::{BTreeMap, BTreeSet};

use rexi_core::grammar::well_known::{
    XML_LOCAL_NAMES, XML_NS, XSD_LOCAL_NAMES, XSD_NS, XSI_LOCAL_NAMES, XSI_NS,
};
use rexi_core::grammar::{GrammarSetBuf, Span, UriEntry};
use rexi_core::{QNameId, UriId};

use super::event_codes::QNameNames;
use crate::xsd::model::Model;

/// One URI with its starting prefixes and local names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UriPartition {
    /// The namespace URI; `""` means no namespace.
    pub uri: String,
    /// The prefixes this partition starts with.
    pub prefixes: Vec<String>,
    /// The local names this partition starts with, in sorted order
    /// ([EXI D.3](https://www.w3.org/TR/exi/#initialLocalNames)).
    pub local_names: Vec<String>,
}

/// The starting string table. The index of a URI in `uris` is its URI id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringTable {
    /// The URI partitions, in URI id order.
    pub uris: Vec<UriPartition>,
}

impl StringTable {
    /// Builds the partitions.
    ///
    /// The URIs are the four fixed ones first: `""` (no namespace), the XML
    /// namespace, the XSI namespace and the XSD namespace
    /// ([D.1](https://www.w3.org/TR/exi/#initialUriValues)). Then come every schema
    /// namespace, sorted by code point.
    ///
    /// Only the first three URIs have a prefix: `""`, `xml` and `xsi`
    /// ([D.2](https://www.w3.org/TR/exi/#initialPrefixValues)).
    ///
    /// The local names of each URI are all element, attribute and named type names
    /// in that namespace, sorted by code point. The XML, XSI and XSD URIs also
    /// start with their fixed names
    /// ([D.3](https://www.w3.org/TR/exi/#initialLocalNames)). Built-in XSD type
    /// names come only from that fixed list.
    pub fn build(model: &Model) -> Self {
        let mut names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for ns in &model.namespaces {
            names.entry(ns.clone()).or_default();
        }
        for e in &model.elements {
            names
                .entry(e.qname.ns.clone())
                .or_default()
                .insert(e.qname.local.clone());
        }
        for a in &model.attributes {
            names
                .entry(a.qname.ns.clone())
                .or_default()
                .insert(a.qname.local.clone());
        }
        for t in &model.types {
            if let Some(q) = &t.name
                && q.ns != XSD_NS
            {
                names
                    .entry(q.ns.clone())
                    .or_default()
                    .insert(q.local.clone());
            }
        }
        let take = |names: &mut BTreeMap<String, BTreeSet<String>>, uri: &str, fixed: &[&str]| {
            let mut set: BTreeSet<String> = fixed.iter().map(|s| s.to_string()).collect();
            if let Some(extra) = names.remove(uri) {
                set.extend(extra);
            }
            set.into_iter().collect::<Vec<_>>()
        };
        let mut uris = vec![
            UriPartition {
                uri: String::new(),
                prefixes: vec![String::new()],
                local_names: take(&mut names, "", &[]),
            },
            UriPartition {
                uri: XML_NS.to_string(),
                prefixes: vec!["xml".to_string()],
                local_names: take(&mut names, XML_NS, &XML_LOCAL_NAMES),
            },
            UriPartition {
                uri: XSI_NS.to_string(),
                prefixes: vec!["xsi".to_string()],
                local_names: take(&mut names, XSI_NS, &XSI_LOCAL_NAMES),
            },
            UriPartition {
                uri: XSD_NS.to_string(),
                prefixes: Vec::new(),
                local_names: take(&mut names, XSD_NS, &XSD_LOCAL_NAMES),
            },
        ];
        for (uri, locals) in names {
            uris.push(UriPartition {
                uri,
                prefixes: Vec::new(),
                local_names: locals.into_iter().collect(),
            });
        }
        StringTable { uris }
    }

    /// The URI id of `uri`, if the table has it.
    pub fn uri_id(&self, uri: &str) -> Option<UriId> {
        self.uris
            .iter()
            .position(|u| u.uri == uri)
            .map(|i| i as UriId)
    }

    /// The qname id of `{ns}local`, if the table has both the URI and the name.
    pub fn qname(&self, ns: &str, local: &str) -> Option<QNameId> {
        let uri = self.uri_id(ns)?;
        let l = self.uris[uri as usize]
            .local_names
            .iter()
            .position(|n| n == local)?;
        Some(QNameId::new(uri, l as u32))
    }

    /// The URI behind a URI id, or `""` for an unknown id.
    pub fn uri_str(&self, u: UriId) -> &str {
        self.uris.get(u as usize).map_or("", |p| p.uri.as_str())
    }

    /// The local name behind a qname id, or `""` for an unknown id.
    pub fn local_name(&self, q: QNameId) -> &str {
        self.uris
            .get(q.uri as usize)
            .and_then(|p| p.local_names.get(q.local as usize))
            .map_or("", String::as_str)
    }

    /// Writes the partitions into the grammar tables. Each string is stored once
    /// (interned), and each URI entry records the span of its prefixes and local
    /// names.
    pub fn write_into(&self, buf: &mut GrammarSetBuf) {
        for p in &self.uris {
            let uri = buf.intern(&p.uri);
            let prefixes_start = buf.prefixes.len() as u32;
            for pfx in &p.prefixes {
                let s = buf.intern(pfx);
                buf.prefixes.push(s);
            }
            let locals_start = buf.local_names.len() as u32;
            for l in &p.local_names {
                let s = buf.intern(l);
                buf.local_names.push(s);
            }
            buf.uris.push(UriEntry {
                uri,
                prefixes: Span::new(prefixes_start, p.prefixes.len() as u32),
                local_names: Span::new(locals_start, p.local_names.len() as u32),
            });
        }
    }
}

impl QNameNames for StringTable {
    fn local_name(&self, q: QNameId) -> &str {
        StringTable::local_name(self, q)
    }
    fn uri(&self, u: UriId) -> &str {
        self.uri_str(u)
    }
}
