//! The decoder handle. It hands decoded events to a struct of callbacks, like
//! exip's `ContentHandler`. It hands over one event per `rexi_decoder_next` call,
//! or all of them with `rexi_decode`. The events are those of
//! [EXI 4](https://www.w3.org/TR/exi/#streams).

use std::ffi::c_int;
use std::ffi::c_void;
use std::io::Cursor;

use rexi_core::{DecodeLimits, Decoder, OwnedEvent};

use crate::limits::RexiLimits;
use crate::options::RexiOptions;
use crate::status::{Fail, RexiStatus, guard};
use crate::tables::{RexiTables, SharedTables, tables_ref};
use crate::value::{RexiStr, RexiValue, Scratch, output};

/// Callbacks for decoded events. Any of them may be NULL; that event is then
/// skipped. Each one returns 0 to keep going. A nonzero return stops decoding with
/// `REXI_STATUS_ERR_ABORTED`, and the decoder can not be used after that.
///
/// Every callback gets the `user` pointer that was passed to `rexi_decoder_next`
/// or `rexi_decode`, unchanged. rexi owns the `RexiStr` strings and `RexiValue`
/// values passed in. They are valid only during the call; copy them to keep them.
/// `uri` is `""` for a name with no namespace.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RexiContentHandler {
    /// SD: the stream has started ([EXI 4](https://www.w3.org/TR/exi/#streams)).
    pub start_document: Option<unsafe extern "C" fn(user: *mut c_void) -> c_int>,
    /// ED: the stream is complete.
    pub end_document: Option<unsafe extern "C" fn(user: *mut c_void) -> c_int>,
    /// SE: an element starts, with its namespace `uri` and `local` name.
    pub start_element:
        Option<unsafe extern "C" fn(uri: RexiStr, local: RexiStr, user: *mut c_void) -> c_int>,
    /// EE: the element that started last ends.
    pub end_element: Option<unsafe extern "C" fn(user: *mut c_void) -> c_int>,
    /// AT: an attribute with its name and its typed `value`. `xsi:type` and
    /// `xsi:nil` come through `xsi_type` and `xsi_nil` instead.
    pub attribute: Option<
        unsafe extern "C" fn(
            uri: RexiStr,
            local: RexiStr,
            value: *const RexiValue,
            user: *mut c_void,
        ) -> c_int,
    >,
    /// CH: text content of an element, as a typed `value`.
    pub characters:
        Option<unsafe extern "C" fn(value: *const RexiValue, user: *mut c_void) -> c_int>,
    /// AT(xsi:type): the element's type is cast to the named type
    /// ([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).
    pub xsi_type:
        Option<unsafe extern "C" fn(uri: RexiStr, local: RexiStr, user: *mut c_void) -> c_int>,
    /// AT(xsi:nil): `nil` is true when the element is nil (has no content).
    pub xsi_nil: Option<unsafe extern "C" fn(nil: bool, user: *mut c_void) -> c_int>,
}

/// An EXI decoder that reads its own copy of the input. C sees only an opaque
/// pointer.
pub struct RexiDecoder {
    inner: Decoder<Cursor<Vec<u8>>, SharedTables>,
    done: bool,
    failed: Option<RexiStatus>,
    uri: Scratch,
    local: Scratch,
    lexical: Scratch,
}

/// Creates a decoder over a copy of `data` and reads the EXI header
/// ([EXI 5](https://www.w3.org/TR/exi/#header)). `out_of_band` gives the options
/// when the header does not carry them. It may be NULL.
///
/// rexi copies `data`, so the caller may free it as soon as this call returns.
/// `out_of_band` is also only read during the call. The decoder keeps its own
/// reference to `tables`, so the tables handle may be freed first. On success
/// `*out` gets a new decoder that the caller owns and must release with
/// `rexi_decoder_free`. On failure `*out` is not written.
///
/// The decoder uses the default limits (`rexi_limits_default()`). Use
/// `rexi_decoder_new_with_limits` to pick others.
///
/// # Safety
/// - `tables` is a live tables handle. NULL returns `ErrNullArgument`.
/// - `data` points to `len` readable bytes. `len` is a count of bytes. `data`
///   may be NULL only when `len` is 0.
/// - `out_of_band` is NULL or points to a valid `RexiOptions`.
/// - `out` points to writable storage for one pointer. NULL returns
///   `ErrNullArgument`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_decoder_new(
    tables: *const RexiTables,
    data: *const u8,
    len: usize,
    out_of_band: *const RexiOptions,
    out: *mut *mut RexiDecoder,
) -> RexiStatus {
    // SAFETY: the same contract, with NULL limits.
    unsafe { rexi_decoder_new_with_limits(tables, data, len, out_of_band, std::ptr::null(), out) }
}

/// Like `rexi_decoder_new`, with limits on what the stream may ask for. `limits`
/// is only read during the call. NULL means the defaults
/// (`rexi_limits_default()`). A stream that goes over a limit fails with
/// `REXI_STATUS_ERR_LIMIT`, here (while reading the header) or later in
/// `rexi_decoder_next`.
///
/// # Safety
/// The same rules as for `rexi_decoder_new`, and `limits` is NULL or points to a
/// valid `RexiLimits`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_decoder_new_with_limits(
    tables: *const RexiTables,
    data: *const u8,
    len: usize,
    out_of_band: *const RexiOptions,
    limits: *const RexiLimits,
    out: *mut *mut RexiDecoder,
) -> RexiStatus {
    guard(|| {
        // SAFETY: per the contract.
        let tables = unsafe { tables_ref(tables) }?;
        if out.is_null() {
            return Err(Fail::null("out"));
        }
        let bytes = if len == 0 {
            Vec::new()
        } else if data.is_null() {
            return Err(Fail::null("data"));
        } else {
            // SAFETY: `data` points to `len` readable bytes per the contract.
            unsafe { std::slice::from_raw_parts(data, len) }.to_vec()
        };
        // SAFETY: per the contract.
        let oob = match unsafe { out_of_band.as_ref() } {
            // SAFETY: per the contract.
            Some(o) => Some(unsafe { o.core() }?.0),
            None => None,
        };
        // SAFETY: per the contract.
        let limits =
            unsafe { limits.as_ref() }.map_or_else(DecodeLimits::default, RexiLimits::core);
        let inner = Decoder::with_limits(Cursor::new(bytes), tables.tables.clone(), oob, limits)?;
        let handle = Box::new(RexiDecoder {
            inner,
            done: false,
            failed: None,
            uri: Scratch::default(),
            local: Scratch::default(),
            lexical: Scratch::default(),
        });
        // SAFETY: `out` is non-null and writable per the contract.
        unsafe { *out = Box::into_raw(handle) };
        Ok(RexiStatus::Ok)
    })
}

fn call(rc: c_int) -> Result<(), Fail> {
    if rc == 0 {
        Ok(())
    } else {
        Err(Fail::new(
            RexiStatus::ErrAborted,
            format!("content handler returned {rc}"),
        ))
    }
}

impl RexiDecoder {
    /// Decodes one event and hands it to the matching callback. Returns `End`
    /// once `ED` has been handed over, and on every call after that.
    ///
    /// The text form of an attribute or character value (from `format_last`) is
    /// built before the callback is checked. So it is built, and can fail, even
    /// when that callback is NULL.
    ///
    /// # Safety
    /// The handler's callbacks are safe to call with `user`.
    unsafe fn step(
        &mut self,
        h: &RexiContentHandler,
        user: *mut c_void,
    ) -> Result<RexiStatus, Fail> {
        if self.done {
            return Ok(RexiStatus::End);
        }
        let ev = {
            let Some(ev) = self.inner.next_event()? else {
                self.done = true;
                return Ok(RexiStatus::End);
            };
            OwnedEvent::from_event(&ev).ok_or_else(|| {
                Fail::new(
                    RexiStatus::ErrInternal,
                    format!("decoder produced {} event", ev.kind_name()),
                )
            })?
        };
        // SAFETY (all callback calls below): the caller promises the callbacks are
        // safe to call. The strings and values passed live in `self` (or in `ev`)
        // for the whole call.
        unsafe {
            match ev {
                OwnedEvent::StartDocument => {
                    if let Some(f) = h.start_document {
                        call(f(user))?;
                    }
                }
                OwnedEvent::EndDocument => {
                    self.done = true;
                    if let Some(f) = h.end_document {
                        call(f(user))?;
                    }
                }
                OwnedEvent::StartElement { uri, local } => {
                    if let Some(f) = h.start_element {
                        call(f(self.uri.set(&uri), self.local.set(&local), user))?;
                    }
                }
                OwnedEvent::EndElement => {
                    if let Some(f) = h.end_element {
                        call(f(user))?;
                    }
                }
                OwnedEvent::Attribute { uri, local, value } => {
                    let lexical = self.inner.format_last(&value.as_value())?;
                    if let Some(f) = h.attribute {
                        let v = output(&value, self.lexical.set(&lexical));
                        call(f(self.uri.set(&uri), self.local.set(&local), &v, user))?;
                    }
                }
                OwnedEvent::Characters(value) => {
                    let lexical = self.inner.format_last(&value.as_value())?;
                    if let Some(f) = h.characters {
                        let v = output(&value, self.lexical.set(&lexical));
                        call(f(&v, user))?;
                    }
                }
                OwnedEvent::XsiType { uri, local } => {
                    if let Some(f) = h.xsi_type {
                        call(f(self.uri.set(&uri), self.local.set(&local), user))?;
                    }
                }
                OwnedEvent::XsiNil(nil) => {
                    if let Some(f) = h.xsi_nil {
                        call(f(nil, user))?;
                    }
                }
                // An event kind added to rexi-core later, which C has no callback for.
                _ => {
                    return Err(Fail::new(
                        RexiStatus::ErrUnsupported,
                        "event kind not known to the C API",
                    ));
                }
            }
        }
        Ok(RexiStatus::Ok)
    }
}

/// Decodes the next event and hands it to `handler`. Returns `REXI_STATUS_OK`
/// after handing over one event, and `REXI_STATUS_END` once the document is
/// complete. After an error (including a callback that returned nonzero) the
/// decoder can not be used; later calls return `ErrState`.
///
/// # Safety
/// - `dec` is a live decoder handle. NULL returns `ErrNullArgument`.
/// - `handler` points to a valid `RexiContentHandler`. NULL returns
///   `ErrNullArgument`. It is only read during the call.
/// - Every non-NULL callback is safe to call with `user`. rexi never reads or
///   writes `user` itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_decoder_next(
    dec: *mut RexiDecoder,
    handler: *const RexiContentHandler,
    user: *mut c_void,
) -> RexiStatus {
    guard(|| {
        // SAFETY: per the contract.
        let d = unsafe { dec.as_mut() }.ok_or_else(|| Fail::null("decoder"))?;
        // SAFETY: per the contract.
        let h = unsafe { handler.as_ref() }.ok_or_else(|| Fail::null("handler"))?;
        if let Some(status) = d.failed {
            return Err(Fail::new(
                RexiStatus::ErrState,
                format!("decoder failed earlier ({status:?}); create a new one"),
            ));
        }
        // SAFETY: per the contract.
        unsafe { d.step(h, user) }.inspect_err(|f| d.failed = Some(f.status))
    })
}

/// Releases a decoder. NULL is ignored. This works at any point, finished or not.
///
/// # Safety
/// `dec` is NULL or a live decoder handle, and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_decoder_free(dec: *mut RexiDecoder) {
    if !dec.is_null() {
        // SAFETY: allocated by `rexi_decoder_new`, freed once.
        drop(unsafe { Box::from_raw(dec) });
    }
}

/// Decodes a whole stream and hands every event to `handler`. It makes a decoder,
/// calls `rexi_decoder_next` until the end or an error, and frees the decoder.
/// Returns `REXI_STATUS_OK` (never `REXI_STATUS_END`) when the whole document was
/// decoded, or the first error. It uses the default limits.
///
/// # Safety
/// The same rules as for `rexi_decoder_new` and `rexi_decoder_next`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_decode(
    tables: *const RexiTables,
    data: *const u8,
    len: usize,
    out_of_band: *const RexiOptions,
    handler: *const RexiContentHandler,
    user: *mut c_void,
) -> RexiStatus {
    // SAFETY: the same contract, with NULL limits.
    unsafe {
        rexi_decode_with_limits(
            tables,
            data,
            len,
            out_of_band,
            std::ptr::null(),
            handler,
            user,
        )
    }
}

/// Like `rexi_decode`, with limits on what the stream may ask for. NULL `limits`
/// means the defaults. See `rexi_decoder_new_with_limits`.
///
/// # Safety
/// The same rules as for `rexi_decoder_new_with_limits` and `rexi_decoder_next`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexi_decode_with_limits(
    tables: *const RexiTables,
    data: *const u8,
    len: usize,
    out_of_band: *const RexiOptions,
    limits: *const RexiLimits,
    handler: *const RexiContentHandler,
    user: *mut c_void,
) -> RexiStatus {
    let mut dec = std::ptr::null_mut();
    // SAFETY: per the contract.
    let status =
        unsafe { rexi_decoder_new_with_limits(tables, data, len, out_of_band, limits, &mut dec) };
    if status != RexiStatus::Ok {
        return status;
    }
    let status = loop {
        // SAFETY: `dec` is live; the rest per the contract.
        match unsafe { rexi_decoder_next(dec, handler, user) } {
            RexiStatus::Ok => {}
            RexiStatus::End => break RexiStatus::Ok,
            err => break err,
        }
    };
    // Freeing does not change the last-error message, so it still describes the
    // error (if any) that ended the loop.
    // SAFETY: `dec` is live and not used afterwards.
    unsafe { rexi_decoder_free(dec) };
    status
}
