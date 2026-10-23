//! The encoder handle: one call per EXI event, like exip's `EXISerializer`.
//!
//! The events and the order they must come in are those of
//! [EXI 4](https://www.w3.org/TR/exi/#streams): `SD`, then one root `SE`, its
//! attributes and content, the matching `EE`, then `ED`. A typical session is
//! `rexi_encoder_new`, the event calls, `rexi_end_document`, then
//! `rexi_encoder_finish` to get the bytes. To give up early, call
//! `rexi_encoder_free` instead of `rexi_encoder_finish`.
//!
//! Every event call needs a live encoder handle. A NULL handle returns
//! `ErrNullArgument`. A NULL string argument also returns `ErrNullArgument`, and a
//! string that is not UTF-8 returns `ErrInvalidArgument`. rexi only reads string
//! and value arguments during the call; the caller keeps ownership of them.

use std::ffi::c_char;

use rexi_core::{Encoder, ExiError, Value};

use crate::options::RexiOptions;
use crate::status::{Fail, RexiStatus, c_str, guard};
use crate::tables::{RexiTables, SharedTables, tables_ref};
use crate::value::RexiValue;

/// An EXI encoder that writes into a growable buffer. C sees only an opaque
/// pointer.
pub struct RexiEncoder {
    inner: Option<Encoder<Vec<u8>, SharedTables>>,
    /// Status of the first failure. Once set, later calls fail with `ErrState`
    /// and name this status in their message.
    failed: Option<RexiStatus>,
}

/// Bytes owned by rexi: `len` bytes at `data`. Release them with
/// `rexi_buffer_free`, and pass the struct back unchanged. When `len` is 0,
/// `data` may be a non-NULL pointer that must not be read.
#[repr(C)]
#[derive(Debug)]
pub struct RexiBuffer {
    /// Start of the bytes. Do not free it yourself.
    pub data: *mut u8,
    /// Number of bytes at `data`.
    pub len: usize,
}

/// Creates an encoder and writes the EXI header
/// ([EXI 5](https://www.w3.org/TR/exi/#header)). If `options->include_options` is
/// set, the options document goes into the header too.
///
/// On success `*out` gets a new encoder that the caller owns. Release it with
/// `rexi_encoder_finish` or `rexi_encoder_free`. On failure `*out` is not written.
/// The encoder keeps its own reference to `tables`, so the tables handle may be
/// freed first. `options` is only read during the call.
///
/// # Safety
/// - `tables` is a live tables handle. NULL returns `ErrNullArgument`.
/// - `options` points to a valid `RexiOptions` (see its rules for `schema_id`).
///   NULL returns `ErrNullArgument`.
/// - `out` points to writable storage for one pointer. NULL returns
///   `ErrNullArgument`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_encoder_new(
    tables: *const RexiTables,
    options: *const RexiOptions,
    out: *mut *mut RexiEncoder,
) -> RexiStatus {
    guard(|| {
        // SAFETY: per the contract.
        let tables = unsafe { tables_ref(tables) }?;
        // SAFETY: per the contract.
        let options = unsafe { options.as_ref() }.ok_or_else(|| Fail::null("options"))?;
        if out.is_null() {
            return Err(Fail::null("out"));
        }
        // SAFETY: per the contract.
        let (opts, header) = unsafe { options.core() }?;
        let enc = Encoder::new(Vec::new(), tables.tables.clone(), opts, header)?;
        let handle = Box::new(RexiEncoder {
            inner: Some(enc),
            failed: None,
        });
        // SAFETY: `out` is non-null and writable per the contract.
        unsafe { *out = Box::into_raw(handle) };
        Ok(RexiStatus::Ok)
    })
}

/// Runs one encoder operation. On failure it marks the handle as failed
/// ("poisons" it), so every later call returns `ErrState`. A finished handle
/// also returns `ErrState`.
///
/// # Safety
/// `enc` is NULL or a live encoder handle.
unsafe fn with_encoder(
    enc: *mut RexiEncoder,
    f: impl FnOnce(&mut Encoder<Vec<u8>, SharedTables>) -> Result<(), Fail>,
) -> RexiStatus {
    guard(|| {
        // SAFETY: per the contract.
        let handle = unsafe { enc.as_mut() }.ok_or_else(|| Fail::null("encoder"))?;
        if let Some(status) = handle.failed {
            return Err(Fail::new(
                RexiStatus::ErrState,
                format!("encoder failed earlier ({status:?}); create a new one"),
            ));
        }
        let Some(inner) = handle.inner.as_mut() else {
            return Err(Fail::new(RexiStatus::ErrState, "encoder already finished"));
        };
        match f(inner) {
            Ok(()) => Ok(RexiStatus::Ok),
            Err(fail) => {
                handle.failed = Some(fail.status);
                Err(fail)
            }
        }
    })
}

fn exi(r: Result<(), ExiError>) -> Result<(), Fail> {
    r.map_err(Fail::from)
}

/// Start Document (`SD`), the first event of every stream.
///
/// # Safety
/// `enc` is a live encoder handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_start_document(enc: *mut RexiEncoder) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe { with_encoder(enc, |e| exi(e.start_document())) }
}

/// End Document (`ED`), the last event. Call `rexi_encoder_finish` after it.
///
/// # Safety
/// `enc` is a live encoder handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_end_document(enc: *mut RexiEncoder) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe { with_encoder(enc, |e| exi(e.end_document())) }
}

/// Start Element (`SE`) named `{uri}local`. `uri` is the namespace name, or `""`
/// for no namespace; `local` is the local name. The pair must name an element the
/// grammar allows here, or the call fails (usually with `ErrGrammar`).
///
/// # Safety
/// `enc` is a live encoder handle; `uri` and `local` are NUL-terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_start_element(
    enc: *mut RexiEncoder,
    uri: *const c_char,
    local: *const c_char,
) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe {
        with_encoder(enc, |e| {
            let (u, l) = (c_str(uri, "uri")?, c_str(local, "local")?);
            exi(e.start_element(u, l))
        })
    }
}

/// End Element (`EE`) for the element started last.
///
/// # Safety
/// `enc` is a live encoder handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_end_element(enc: *mut RexiEncoder) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe { with_encoder(enc, |e| exi(e.end_element())) }
}

/// Attribute (`AT`) named `{uri}local`, with a value. `uri` is `""` for no
/// namespace.
///
/// # Safety
/// `enc` is a live encoder handle; `uri` and `local` are NUL-terminated UTF-8;
/// `value` points to a `RexiValue` that follows its input rules. A NULL `value`
/// returns `ErrNullArgument`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_attribute(
    enc: *mut RexiEncoder,
    uri: *const c_char,
    local: *const c_char,
    value: *const RexiValue,
) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe {
        with_encoder(enc, |e| {
            let (u, l) = (c_str(uri, "uri")?, c_str(local, "local")?);
            let v = value.as_ref().ok_or_else(|| Fail::null("value"))?.input()?;
            exi(e.attribute(u, l, &v.as_value()))
        })
    }
}

/// Attribute (`AT`) named `{uri}local`, with its value as text. The text is
/// parsed with the attribute's datatype.
///
/// # Safety
/// `enc` is a live encoder handle; `uri`, `local` and `lexical` are
/// NUL-terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_attribute_lexical(
    enc: *mut RexiEncoder,
    uri: *const c_char,
    local: *const c_char,
    lexical: *const c_char,
) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe {
        with_encoder(enc, |e| {
            let (u, l) = (c_str(uri, "uri")?, c_str(local, "local")?);
            let v = c_str(lexical, "lexical")?;
            exi(e.attribute(u, l, &Value::Lexical(v)))
        })
    }
}

/// Characters (`CH`), that is, text content, with a value.
///
/// # Safety
/// `enc` is a live encoder handle; `value` points to a `RexiValue` that follows
/// its input rules. A NULL `value` returns `ErrNullArgument`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_characters(
    enc: *mut RexiEncoder,
    value: *const RexiValue,
) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe {
        with_encoder(enc, |e| {
            let v = value.as_ref().ok_or_else(|| Fail::null("value"))?.input()?;
            exi(e.characters(&v.as_value()))
        })
    }
}

/// Characters (`CH`) as text, parsed with the content's datatype. Calls made one
/// right after another are joined into one value, the same way XML character data
/// is.
///
/// # Safety
/// `enc` is a live encoder handle; `lexical` is NUL-terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_characters_lexical(
    enc: *mut RexiEncoder,
    lexical: *const c_char,
) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe {
        with_encoder(enc, |e| {
            let v = c_str(lexical, "lexical")?;
            exi(e.characters(&Value::Lexical(v)))
        })
    }
}

/// The `xsi:type` attribute, naming the type `{uri}local`. `xsi:type` tells the
/// reader to use that schema type for this element instead of the declared one
/// ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)). It must
/// come right after the start tag.
///
/// # Safety
/// `enc` is a live encoder handle; `uri` and `local` are NUL-terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_xsi_type(
    enc: *mut RexiEncoder,
    uri: *const c_char,
    local: *const c_char,
) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe {
        with_encoder(enc, |e| {
            let (u, l) = (c_str(uri, "uri")?, c_str(local, "local")?);
            exi(e.xsi_type(u, l))
        })
    }
}

/// The `xsi:nil` attribute. `xsi:nil="true"` marks the element as having no
/// content. In strict mode it is allowed only on elements the schema marks as
/// nillable ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).
/// It must follow the start tag, after `xsi:type` if there is one.
///
/// # Safety
/// `enc` is a live encoder handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_xsi_nil(enc: *mut RexiEncoder, nil: bool) -> RexiStatus {
    // SAFETY: per the contract.
    unsafe { with_encoder(enc, |e| exi(e.xsi_nil(nil))) }
}

/// Finishes the stream (call it after `rexi_end_document`) and hands over the
/// bytes. On success `*out` gets the bytes, which the caller owns and must release
/// with `rexi_buffer_free`. On failure `*out` is not written.
///
/// The encoder is freed in every case, even on error, so never use or free it
/// again. A handle that failed earlier returns `ErrState`.
///
/// # Safety
/// - `enc` is a live encoder handle. NULL returns `ErrNullArgument`.
/// - `out` points to a writable `RexiBuffer`. NULL returns `ErrNullArgument`, and
///   the encoder is still freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_encoder_finish(
    enc: *mut RexiEncoder,
    out: *mut RexiBuffer,
) -> RexiStatus {
    guard(|| {
        if enc.is_null() {
            return Err(Fail::null("encoder"));
        }
        // SAFETY: allocated by `rexi_encoder_new`, ownership returns here.
        let handle = unsafe { Box::from_raw(enc) };
        if out.is_null() {
            return Err(Fail::null("out"));
        }
        if let Some(status) = handle.failed {
            return Err(Fail::new(
                RexiStatus::ErrState,
                format!("encoder failed earlier ({status:?})"),
            ));
        }
        let Some(inner) = handle.inner else {
            return Err(Fail::new(RexiStatus::ErrState, "encoder already finished"));
        };
        let bytes = inner.finish()?.into_boxed_slice();
        let len = bytes.len();
        let data = Box::into_raw(bytes).cast::<u8>();
        // SAFETY: `out` is non-null and writable per the contract.
        unsafe { *out = RexiBuffer { data, len } };
        Ok(RexiStatus::Ok)
    })
}

/// Throws away an unfinished encoder and its bytes. NULL is ignored. Do not call
/// it on an encoder already passed to `rexi_encoder_finish`.
///
/// # Safety
/// `enc` is NULL or a live encoder handle, and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_encoder_free(enc: *mut RexiEncoder) {
    if !enc.is_null() {
        // SAFETY: allocated by `rexi_encoder_new`, freed once.
        drop(unsafe { Box::from_raw(enc) });
    }
}

/// Releases bytes returned by `rexi_encoder_finish`. A NULL `data` is ignored.
///
/// # Safety
/// `buffer` came from `rexi_encoder_finish` with `data` and `len` unchanged, and
/// is freed only once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_buffer_free(buffer: RexiBuffer) {
    if buffer.data.is_null() {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(buffer.data, buffer.len);
    // SAFETY: created by `Box::into_raw` of a boxed slice of this length.
    drop(unsafe { Box::from_raw(slice) });
}
