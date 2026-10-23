//! Decoder limits against hostile streams. Each one used to hang, run out of
//! memory or overflow the stack. Now each stops quickly with `LimitExceeded`,
//! naming the right limit, and raising that limit lets a stream just over the
//! default through.

use std::time::{Duration, Instant};

use rexi_core::{
    Alignment, DecodeLimits, Decoder, Encoder, Event, ExiError, ExiOptions, GrammarSetBuf,
    HeaderConfig, Limit, QName,
};

/// Every check must fail fast. This is loose, so a slow debug build still passes.
const QUICK: Duration = Duration::from_secs(5);

fn compile(xsd: &str) -> GrammarSetBuf {
    rexi_schema::compile_str(xsd, None).unwrap_or_else(|e| panic!("{e}"))
}

fn byte_aligned() -> ExiOptions {
    ExiOptions::strict().with_alignment(Alignment::ByteAligned)
}

/// Pulls every event out of `bytes` without keeping them. Returns the event count.
fn drain(
    bytes: &[u8],
    t: &GrammarSetBuf,
    options: ExiOptions,
    limits: DecodeLimits,
) -> Result<u64, ExiError> {
    let mut dec = Decoder::with_limits(bytes, t, Some(options), limits)?;
    let mut n = 0u64;
    while dec.next_event()?.is_some() {
        n += 1;
    }
    Ok(n)
}

fn assert_limit(r: Result<u64, ExiError>, limit: Limit, max: u64) {
    assert_eq!(
        r,
        Err(ExiError::LimitExceeded { limit, max }),
        "expected {limit} over {max}"
    );
}

/// A list of a one-value enumeration: each item is 0 bits, so the count alone
/// decides the work.
const ZERO_BIT_LIST: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:simpleType name="one"><xs:restriction base="xs:string"><xs:enumeration value="a"/></xs:restriction></xs:simpleType>
  <xs:element name="r"><xs:simpleType><xs:list itemType="one"/></xs:simpleType></xs:element>
</xs:schema>"#;

/// Header, `SE(r)`, then the item count as an Unsigned Integer (byte-aligned).
fn zero_bit_list(count: u64) -> Vec<u8> {
    let mut b = vec![0x80, 0x00];
    let mut v = count;
    loop {
        let low = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            b.push(low);
            return b;
        }
        b.push(low | 0x80);
    }
}

#[test]
fn zero_bit_list_stops_at_list_items() {
    let t = compile(ZERO_BIT_LIST);
    // 7 bytes that ask for about four billion items.
    let evil = [0x80, 0x00, 0xff, 0xff, 0xff, 0xff, 0x0f];
    assert_eq!(zero_bit_list(0xFFFF_FFFF), evil);
    let start = Instant::now();
    assert_limit(
        drain(&evil, &t, byte_aligned(), DecodeLimits::default()),
        Limit::ListItems,
        1_000_000,
    );
    assert!(start.elapsed() < QUICK);

    // One item over the default, then let through by raising the limit.
    let just_over = zero_bit_list(1_000_001);
    assert_limit(
        drain(&just_over, &t, byte_aligned(), DecodeLimits::default()),
        Limit::ListItems,
        1_000_000,
    );
    let raised = DecodeLimits::default().with_max_list_items(Some(1_000_001));
    assert!(drain(&just_over, &t, byte_aligned(), raised).is_ok());
}

/// `<a>` must contain another `<a>`: every start tag costs 0 bits.
const REQUIRED_RECURSION: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="a"><xs:complexType><xs:sequence><xs:element ref="a"/></xs:sequence></xs:complexType></xs:element>
</xs:schema>"#;

/// `<a>` may contain another `<a>`.
const OPTIONAL_RECURSION: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="a"><xs:complexType><xs:sequence><xs:element ref="a" minOccurs="0"/></xs:sequence></xs:complexType></xs:element>
</xs:schema>"#;

/// `depth` nested `<a>` elements.
fn nested(t: &GrammarSetBuf, depth: usize) -> Vec<u8> {
    let mut enc = Encoder::new(Vec::new(), t, ExiOptions::strict(), HeaderConfig::default())
        .unwrap_or_else(|e| panic!("{e}"));
    let push = |enc: &mut Encoder<Vec<u8>, &GrammarSetBuf>, ev: &Event<'_>| {
        enc.push(ev).unwrap_or_else(|e| panic!("{e}"));
    };
    push(&mut enc, &Event::StartDocument);
    for _ in 0..depth {
        push(&mut enc, &Event::StartElement(QName::new("", "a")));
    }
    for _ in 0..depth {
        push(&mut enc, &Event::EndElement);
    }
    push(&mut enc, &Event::EndDocument);
    enc.finish().unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn required_recursion_stops_at_depth() {
    let t = compile(REQUIRED_RECURSION);
    let start = Instant::now();
    assert_limit(
        drain(
            &[0x80, 0x00],
            &t,
            ExiOptions::strict(),
            DecodeLimits::default(),
        ),
        Limit::Depth,
        256,
    );
    assert!(start.elapsed() < QUICK);
    // A higher limit only moves the point where it stops.
    let raised = DecodeLimits::default().with_max_depth(Some(5000));
    assert_limit(
        drain(&[0x80, 0x00], &t, ExiOptions::strict(), raised),
        Limit::Depth,
        5000,
    );
}

#[test]
fn depth_limit_is_open_elements() {
    let t = compile(OPTIONAL_RECURSION);
    let ok = nested(&t, 256);
    assert!(drain(&ok, &t, ExiOptions::strict(), DecodeLimits::default()).is_ok());
    let just_over = nested(&t, 257);
    assert_limit(
        drain(
            &just_over,
            &t,
            ExiOptions::strict(),
            DecodeLimits::default(),
        ),
        Limit::Depth,
        256,
    );
    let raised = DecodeLimits::default().with_max_depth(Some(257));
    assert!(drain(&just_over, &t, ExiOptions::strict(), raised).is_ok());
    // rexi-xml passes the limits through.
    let r =
        rexi_xml::decode_to_xml_with_limits(&just_over[..], &t, Some(ExiOptions::strict()), raised);
    assert!(r.is_ok(), "{r:?}");
    let r = rexi_xml::decode_events(&just_over[..], &t, Some(ExiOptions::strict()));
    assert!(
        matches!(
            r,
            Err(rexi_xml::XmlError::Exi(ExiError::LimitExceeded {
                limit: Limit::Depth,
                ..
            }))
        ),
        "{r:?}"
    );
}

const INTEGERS: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="i" type="xs:integer"/>
  <xs:element name="d" type="xs:decimal"/>
</xs:schema>"#;

/// Byte-aligned `<i>` holding an Integer of `groups` 7-bit groups, all ones.
fn big_integer(groups: usize) -> Vec<u8> {
    // Header, SE(i) (code 1 of d, i), CH (code 0; code 1 would be the xsi:type
    // escape), the sign (positive), then the groups.
    let mut b = vec![0x80, 0x01, 0x00, 0x00];
    b.extend(std::iter::repeat_n(0xFF, groups - 1));
    b.push(0x7F);
    b
}

#[test]
fn huge_integer_stops_at_integer_bits() {
    let t = compile(INTEGERS);
    // About 100 KB of groups.
    let evil = big_integer(100_000);
    let start = Instant::now();
    assert_limit(
        drain(&evil, &t, byte_aligned(), DecodeLimits::default()),
        Limit::IntegerBits,
        4096,
    );
    assert!(start.elapsed() < QUICK);

    // 585 groups are 4095 bits; 586 are 4102.
    assert!(
        drain(
            &big_integer(585),
            &t,
            byte_aligned(),
            DecodeLimits::default()
        )
        .is_ok()
    );
    let just_over = big_integer(586);
    assert_limit(
        drain(&just_over, &t, byte_aligned(), DecodeLimits::default()),
        Limit::IntegerBits,
        4096,
    );
    let raised = DecodeLimits::default().with_max_integer_bits(Some(4102));
    assert!(drain(&just_over, &t, byte_aligned(), raised).is_ok());
    // Decimals read the same groups.
    let mut decimal = vec![0x80, 0x00, 0x00, 0x00];
    decimal.extend(std::iter::repeat_n(0xFF, 10_000));
    decimal.push(0x00);
    assert_limit(
        drain(&decimal, &t, byte_aligned(), DecodeLimits::default()),
        Limit::IntegerBits,
        4096,
    );
}

const STRINGS: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="r"><xs:complexType><xs:sequence>
    <xs:element name="s" type="xs:string" maxOccurs="unbounded"/>
  </xs:sequence></xs:complexType></xs:element>
</xs:schema>"#;

/// Byte-aligned `<r>` with one `<s>` holding `text`, then `hits` more `<s>` that
/// each repeat it through a 3-byte string-table hit. Built from the encoding of
/// the one-`<s>` document: each hit is `SE(s)`, "local hit" and the local id,
/// all zero bytes, placed before the final `EE(r)`.
fn string_hits(t: &GrammarSetBuf, text: &str, hits: usize) -> Vec<u8> {
    let xml = format!("<r><s>{text}</s></r>");
    let mut b = rexi_xml::encode_xml(xml.as_bytes(), t, &byte_aligned(), HeaderConfig::default())
        .unwrap_or_else(|e| panic!("{e}"));
    let end = b.pop();
    assert_eq!(end, Some(0x01));
    b.extend(std::iter::repeat_n(0x00, 3 * hits));
    b.push(0x01);
    b
}

#[test]
fn string_hits_stop_at_string_bytes() {
    let t = compile(STRINGS);
    // The stream really is what it says: SD, SE(r), 3 events per <s>, EE, ED.
    let small = string_hits(&t, "xyz", 2);
    assert_eq!(
        drain(&small, &t, byte_aligned(), DecodeLimits::default()),
        Ok(4 + 3 * 3)
    );
    // 64 KiB once, then 4096 one-hit copies: 4097 * 64 KiB is just over 256 MiB,
    // from a stream of about 76 KB.
    let big = "x".repeat(65536);
    let evil = string_hits(&t, &big, 4096);
    assert!(evil.len() < 80_000);
    let start = Instant::now();
    assert_limit(
        drain(&evil, &t, byte_aligned(), DecodeLimits::default()),
        Limit::StringBytes,
        256 * 1024 * 1024,
    );
    assert!(start.elapsed() < QUICK);
    // 4096 copies in all are exactly 256 MiB, which is allowed.
    let at_limit = string_hits(&t, &big, 4095);
    assert!(drain(&at_limit, &t, byte_aligned(), DecodeLimits::default()).is_ok());
    let raised = DecodeLimits::default().with_max_string_bytes(Some(4097 * 65536));
    assert!(drain(&evil, &t, byte_aligned(), raised).is_ok());
}

#[test]
fn unlimited_turns_every_check_off() {
    let t = compile(ZERO_BIT_LIST);
    let over = zero_bit_list(1_000_001);
    assert!(drain(&over, &t, byte_aligned(), DecodeLimits::unlimited()).is_ok());
    let t = compile(INTEGERS);
    assert!(
        drain(
            &big_integer(586),
            &t,
            byte_aligned(),
            DecodeLimits::unlimited()
        )
        .is_ok()
    );
}
