//! Schemas whose grammars would never end or grow without bound are refused
//! cleanly: circular group references, huge `minOccurs` / `maxOccurs`, and groups
//! that each use the one before more than once (`max_grammar_size`).

use std::time::{Duration, Instant};

use rexi_schema::{SchemaError, SchemaLoadOptions, compile_str, compile_str_with};

fn xs(body: &str) -> String {
    format!(r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">{body}</xs:schema>"#)
}

/// A root element whose content is one `e` element with these occurrence bounds.
fn repeated(min: &str, max: &str) -> String {
    xs(&format!(
        r#"<xs:element name="r"><xs:complexType><xs:sequence>
             <xs:element name="e" type="xs:int" minOccurs="{min}" maxOccurs="{max}"/>
           </xs:sequence></xs:complexType></xs:element>"#
    ))
}

fn invalid(result: Result<impl Sized, SchemaError>) -> String {
    match result {
        Err(SchemaError::Invalid(m)) => m,
        Err(other) => panic!("expected Invalid, got {other}"),
        Ok(_) => panic!("expected Invalid, got Ok"),
    }
}

fn too_large(result: Result<impl Sized, SchemaError>) -> usize {
    match result {
        Err(SchemaError::TooLarge { max }) => max,
        Err(other) => panic!("expected TooLarge, got {other}"),
        Ok(_) => panic!("expected TooLarge, got Ok"),
    }
}

#[test]
fn circular_groups_are_refused() {
    let direct = xs(
        r#"<xs:group name="g"><xs:sequence><xs:group ref="g"/></xs:sequence></xs:group>
        <xs:element name="r"><xs:complexType><xs:group ref="g"/></xs:complexType></xs:element>"#,
    );
    assert!(invalid(compile_str(&direct, None)).contains("circular group reference"));

    let indirect = xs(
        r#"<xs:group name="a"><xs:sequence><xs:group ref="b"/></xs:sequence></xs:group>
        <xs:group name="b"><xs:choice><xs:element name="x"/><xs:group ref="a"/></xs:choice></xs:group>
        <xs:element name="r"><xs:complexType><xs:group ref="a"/></xs:complexType></xs:element>"#,
    );
    assert!(invalid(compile_str(&indirect, None)).contains("circular group reference"));

    // Back to the group through a local element's anonymous type: each copy of the
    // group would make a new copy of the element, without end.
    let through_element = xs(r#"<xs:group name="g"><xs:sequence>
          <xs:element name="d" minOccurs="0"><xs:complexType><xs:group ref="g"/></xs:complexType></xs:element>
        </xs:sequence></xs:group>
        <xs:element name="r"><xs:complexType><xs:group ref="g"/></xs:complexType></xs:element>"#);
    assert!(invalid(compile_str(&through_element, None)).contains("circular group reference"));
}

#[test]
fn huge_occurrence_bounds_are_refused_quickly() {
    let start = Instant::now();
    for (min, max) in [("0", "4000000000"), ("4000000000", "unbounded")] {
        assert_eq!(
            too_large(compile_str(&repeated(min, max), None)),
            SchemaLoadOptions::DEFAULT_MAX_GRAMMAR_SIZE
        );
    }
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn groups_that_double_are_refused_quickly() {
    // g0 holds one element; each g_i uses g_(i-1) twice, so g20 would expand to
    // about a million copies.
    let mut body = String::from(
        r#"<xs:group name="g0"><xs:sequence><xs:element name="x"/></xs:sequence></xs:group>"#,
    );
    for i in 1..=20 {
        let p = i - 1;
        body.push_str(&format!(
            r#"<xs:group name="g{i}"><xs:sequence><xs:group ref="g{p}"/><xs:group ref="g{p}"/></xs:sequence></xs:group>"#
        ));
    }
    body.push_str(r#"<xs:element name="r"><xs:complexType><xs:group ref="g20"/></xs:complexType></xs:element>"#);
    let start = Instant::now();
    too_large(compile_str(&xs(&body), None));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn the_limit_can_be_set() {
    let small = SchemaLoadOptions::default().with_max_grammar_size(Some(1000));
    assert_eq!(
        too_large(compile_str_with(&repeated("0", "600"), None, &small)),
        1000
    );
    assert!(compile_str_with(&repeated("0", "100"), None, &small).is_ok());
    let none = SchemaLoadOptions::default().with_max_grammar_size(None);
    assert!(compile_str_with(&repeated("1", "3"), None, &none).is_ok());
}

#[test]
fn a_long_repeat_is_refused_before_it_uses_up_memory() {
    // 20,000 optional copies are only 40,000 rules, but normalizing them makes
    // about 200 million productions (each copy can skip to every later one). The
    // production count stops it.
    let start = Instant::now();
    too_large(compile_str(&repeated("0", "20000"), None));
    assert!(start.elapsed() < Duration::from_secs(10));
    // A long but sensible repeat still compiles.
    assert!(compile_str(&repeated("0", "2000"), None).is_ok());
}

#[test]
fn legal_recursion_through_groups_still_compiles() {
    // A global element whose type uses a group that refers back to the element
    // (XHTML's div and its flow content), and a named type that does the same
    // (a tree). Global elements and named types are resolved once, so these end.
    let global = xs(r#"<xs:group name="flow"><xs:choice>
          <xs:element ref="div"/><xs:element name="p" type="xs:string"/>
        </xs:choice></xs:group>
        <xs:element name="div"><xs:complexType>
          <xs:group ref="flow" minOccurs="0" maxOccurs="unbounded"/>
        </xs:complexType></xs:element>"#);
    assert!(compile_str(&global, None).is_ok());
    let named = xs(r#"<xs:group name="kids"><xs:sequence>
          <xs:element name="node" type="Node" minOccurs="0" maxOccurs="unbounded"/>
        </xs:sequence></xs:group>
        <xs:complexType name="Node"><xs:group ref="kids"/></xs:complexType>
        <xs:element name="tree" type="Node"/>"#);
    assert!(compile_str(&named, None).is_ok());
}

#[test]
fn a_list_of_itself_or_of_a_list_is_refused() {
    let own = xs(
        r#"<xs:simpleType name="A"><xs:list itemType="A"/></xs:simpleType>
        <xs:element name="r" type="A"/>"#,
    );
    assert!(invalid(compile_str(&own, None)).contains("list type"));
    let of_list = xs(
        r#"<xs:simpleType name="L"><xs:list itemType="xs:NMTOKENS"/></xs:simpleType>
        <xs:element name="r" type="L"/>"#,
    );
    assert!(invalid(compile_str(&of_list, None)).contains("list type"));
    let fine = xs(
        r#"<xs:simpleType name="L"><xs:list itemType="xs:int"/></xs:simpleType>
        <xs:element name="r" type="L"/>"#,
    );
    assert!(compile_str(&fine, None).is_ok());
}
