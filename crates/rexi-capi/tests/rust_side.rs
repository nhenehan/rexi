//! The C entry points called from Rust: tables from generated statics
//! (`RexiTables::from_static`), handles outliving their tables, and the
//! last-error contract.

use std::ffi::{CStr, CString, c_int, c_void};

use rexi::*;
use rexi_tests::generated::order_complex as oc;

fn last_error() -> String {
    // SAFETY: rexi_last_error returns a valid NUL-terminated string.
    unsafe { CStr::from_ptr(rexi_last_error()) }
        .to_string_lossy()
        .into_owned()
}

unsafe extern "C" fn count(_: RexiStr, _: RexiStr, user: *mut c_void) -> c_int {
    // SAFETY: `user` is the `&mut usize` passed below.
    unsafe { *user.cast::<usize>() += 1 };
    0
}

#[test]
fn generated_static_tables_decode_the_reference_and_outlive_their_handle() {
    let case = rexi_tests::cases()
        .into_iter()
        .find(|c| c.name == "order-complex")
        .unwrap_or_else(|| panic!("case missing"));
    let bytes = std::fs::read(case.reference("strict-bp")).unwrap_or_default();
    let tables = Box::into_raw(RexiTables::from_static(&oc::GRAMMARS));
    let opts = rexi_options_strict();
    let mut dec = std::ptr::null_mut();
    // SAFETY: valid handle, buffer and out pointer.
    let s = unsafe { rexi_decoder_new(tables, bytes.as_ptr(), bytes.len(), &opts, &mut dec) };
    assert_eq!(s, RexiStatus::Ok, "{}", last_error());
    // The decoder keeps its own reference to the tables.
    // SAFETY: freed once; not used afterwards.
    unsafe { rexi_tables_free(tables) };
    let handler = RexiContentHandler {
        start_element: Some(count),
        ..RexiContentHandler::default()
    };
    let mut elements = 0usize;
    loop {
        // SAFETY: live decoder, valid handler, `user` is `&mut elements`.
        let s = unsafe { rexi_decoder_next(dec, &handler, (&raw mut elements).cast()) };
        match s {
            RexiStatus::Ok => {}
            RexiStatus::End => break,
            other => panic!("{other:?}: {}", last_error()),
        }
    }
    // SAFETY: live decoder, freed once.
    unsafe { rexi_decoder_free(dec) };
    // Order, Customer, Name, Tier, 2 × (Item, Description, Quantity, Price), Notes.
    assert_eq!(elements, 13);
    assert_eq!(last_error(), "");
}

#[test]
fn encoder_from_xsd_string_and_errors() {
    let xsd = CString::new(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
             <xs:element name="n" type="xs:int"/></xs:schema>"#,
    )
    .unwrap_or_default();
    let mut tables = std::ptr::null_mut();
    // SAFETY: valid strings and out pointer.
    let s = unsafe { rexi_tables_from_xsd_string(xsd.as_ptr(), std::ptr::null(), &mut tables) };
    assert_eq!(s, RexiStatus::Ok, "{}", last_error());
    let opts = rexi_options_strict();
    let mut enc = std::ptr::null_mut();
    // SAFETY: valid handle, options and out pointer.
    assert_eq!(
        unsafe { rexi_encoder_new(tables, &opts, &mut enc) },
        RexiStatus::Ok
    );
    let value = RexiValue {
        kind: REXI_VALUE_INTEGER,
        bool_value: false,
        int_value: -42,
        uint_value: 0,
        double_value: 0.0,
        bytes: std::ptr::null(),
        bytes_len: 0,
        lexical: std::ptr::null(),
        lexical_len: 0,
    };
    // SAFETY: live encoder and valid arguments throughout.
    unsafe {
        assert_eq!(rexi_start_document(enc), RexiStatus::Ok);
        assert_eq!(
            rexi_start_element(enc, c"".as_ptr(), c"n".as_ptr()),
            RexiStatus::Ok
        );
        let bad = RexiValue { kind: 99, ..value };
        assert_eq!(rexi_characters(enc, &bad), RexiStatus::ErrInvalidArgument);
        assert!(last_error().contains("unknown value kind 99"));
        // The handle is poisoned after any failure.
        assert_eq!(rexi_characters(enc, &value), RexiStatus::ErrState);
        let mut out = RexiBuffer {
            data: std::ptr::null_mut(),
            len: 0,
        };
        assert_eq!(rexi_encoder_finish(enc, &mut out), RexiStatus::ErrState);
        assert!(out.data.is_null());
    }

    // A clean run: SD SE(n) CH(-42) EE ED.
    // SAFETY: as above.
    unsafe {
        assert_eq!(rexi_encoder_new(tables, &opts, &mut enc), RexiStatus::Ok);
        rexi_start_document(enc);
        rexi_start_element(enc, c"".as_ptr(), c"n".as_ptr());
        assert_eq!(rexi_characters(enc, &value), RexiStatus::Ok);
        rexi_end_element(enc);
        assert_eq!(rexi_end_document(enc), RexiStatus::Ok);
        let mut out = RexiBuffer {
            data: std::ptr::null_mut(),
            len: 0,
        };
        assert_eq!(rexi_encoder_finish(enc, &mut out), RexiStatus::Ok);
        let bytes = std::slice::from_raw_parts(out.data, out.len).to_vec();
        rexi_buffer_free(out);
        // Same bytes as the XML front-end produces for <n>-42</n>.
        let buf = rexi_schema::compile_str(
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
                 <xs:element name="n" type="xs:int"/></xs:schema>"#,
            None,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let via_xml = rexi_xml::encode_xml(
            b"<n>-42</n>",
            &buf,
            &rexi_core::ExiOptions::strict(),
            rexi_core::HeaderConfig::default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(bytes, via_xml);
        rexi_tables_free(tables);
    }
}
