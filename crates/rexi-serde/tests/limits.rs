//! Decoder limits through serde. A 200,000-deep document used to overflow the
//! stack, first while decoding the tree and then while dropping it.

use rexi_core::{
    DecodeLimits, Encoder, Event, ExiError, ExiOptions, GrammarSetBuf, HeaderConfig, Limit, QName,
};
use rexi_serde::{Error, from_exi, from_exi_with_limits, from_reader_with_limits};
use serde::Deserialize;

/// `<a>` may contain another `<a>`: each level costs one bit.
const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="a"><xs:complexType><xs:sequence><xs:element ref="a" minOccurs="0"/></xs:sequence></xs:complexType></xs:element>
</xs:schema>"#;

#[derive(Deserialize, Debug)]
struct A {}

fn compile() -> GrammarSetBuf {
    rexi_schema::compile_str(XSD, None).unwrap_or_else(|e| panic!("{e}"))
}

/// `depth` nested `<a>` elements.
fn nested(t: &GrammarSetBuf, depth: usize) -> Vec<u8> {
    let mut enc = Encoder::new(Vec::new(), t, ExiOptions::strict(), HeaderConfig::default())
        .unwrap_or_else(|e| panic!("{e}"));
    let mut push = |ev: &Event<'_>| enc.push(ev).unwrap_or_else(|e| panic!("{e}"));
    push(&Event::StartDocument);
    for _ in 0..depth {
        push(&Event::StartElement(QName::new("", "a")));
    }
    for _ in 0..depth {
        push(&Event::EndElement);
    }
    push(&Event::EndDocument);
    enc.finish().unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn deep_document_stops_at_depth() {
    let t = compile();
    let bytes = nested(&t, 200_000);
    let r: Result<A, Error> = from_exi(&bytes, &t, Some(ExiOptions::strict()));
    assert_eq!(
        r.map(drop),
        Err(Error::Exi(ExiError::LimitExceeded {
            limit: Limit::Depth,
            max: 256
        }))
    );
    // Just over the default, let through by raising the limit.
    let bytes = nested(&t, 257);
    let r: Result<A, Error> = from_exi(&bytes, &t, Some(ExiOptions::strict()));
    assert!(r.is_err());
    let raised = DecodeLimits::default().with_max_depth(Some(257));
    let r: Result<A, Error> = from_exi_with_limits(&bytes, &t, Some(ExiOptions::strict()), raised);
    assert!(r.is_ok(), "{r:?}");
    let r: Result<A, Error> =
        from_reader_with_limits(&bytes[..], &t, Some(ExiOptions::strict()), raised);
    assert!(r.is_ok(), "{r:?}");
}

/// With every limit off, the 200,000-deep tree is built and dropped without
/// recursion. This runs in a thread with a 2 MiB stack (the usual size for a
/// test thread), to show it does not depend on a big main-thread stack.
#[test]
fn deep_document_without_limits_does_not_overflow() {
    let t = compile();
    let bytes = nested(&t, 200_000);
    let handle = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let r: Result<A, Error> = from_exi_with_limits(
                &bytes,
                &t,
                Some(ExiOptions::strict()),
                DecodeLimits::unlimited(),
            );
            r.is_ok()
        })
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(handle.join().ok(), Some(true));
}
