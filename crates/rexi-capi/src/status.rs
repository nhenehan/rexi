//! Status codes, the last-error message (one per thread), and the panic guard
//! that every exported function runs inside.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

use rexi_core::ExiError;

/// Result of every `rexi_*` call that can fail. For any value other than
/// `REXI_STATUS_OK` or `REXI_STATUS_END`, `rexi_last_error()` says what went wrong.
/// Errors are negative; the two success values are 0 and 1.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RexiStatus {
    /// The call worked.
    Ok = 0,
    /// Only from `rexi_decoder_next`: the document is complete, and no event was
    /// handed to a callback on this call.
    End = 1,
    /// A required pointer argument was NULL.
    ErrNullArgument = -1,
    /// An argument is badly formed: not UTF-8, out of range, or an unknown kind.
    ErrInvalidArgument = -2,
    /// Reading or writing the bytes failed, or a fixed output buffer is full.
    ErrIo = -3,
    /// The stream ended before the document was complete.
    ErrTruncated = -4,
    /// The EXI header is malformed.
    ErrHeader = -5,
    /// An EXI feature that rexi does not support on purpose (compression,
    /// non-strict mode, ...).
    ErrUnsupported = -6,
    /// The event does not fit the grammar at this point.
    ErrGrammar = -7,
    /// The value does not fit the datatype that the grammar calls for here.
    ErrValue = -8,
    /// The string table (the list of names and values seen so far) does not add up,
    /// or a stream refers to an entry that does not exist
    /// ([EXI 7.3](https://www.w3.org/TR/exi/#stringTable)).
    ErrStringTable = -9,
    /// The schema could not be loaded or compiled.
    ErrSchema = -10,
    /// A bug: a rule that rexi counts on always holding was broken.
    ErrInternal = -11,
    /// A content-handler callback returned a nonzero value.
    ErrAborted = -12,
    /// The handle was used after an earlier error, or after it was finished.
    ErrState = -13,
    /// A Rust panic was caught at the C boundary. This is a bug in rexi.
    ErrPanic = -14,
    /// The stream went over one of the decoder limits (`RexiLimits`): too deep,
    /// a list too long, an integer too big, or too many bytes of strings.
    ErrLimit = -15,
}

/// A failure on its way back to the C caller: the status to return and the
/// message to store for `rexi_last_error()`.
pub(crate) struct Fail {
    pub status: RexiStatus,
    pub message: String,
}

impl Fail {
    pub(crate) fn new(status: RexiStatus, message: impl Into<String>) -> Self {
        Fail {
            status,
            message: message.into(),
        }
    }

    pub(crate) fn null(what: &str) -> Self {
        Fail::new(RexiStatus::ErrNullArgument, format!("{what} is NULL"))
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Fail::new(RexiStatus::ErrInvalidArgument, message)
    }
}

impl From<ExiError> for Fail {
    fn from(e: ExiError) -> Self {
        let status = match &e {
            ExiError::Io(_) => RexiStatus::ErrIo,
            ExiError::Truncated => RexiStatus::ErrTruncated,
            ExiError::Header(_) => RexiStatus::ErrHeader,
            ExiError::Unsupported(_) => RexiStatus::ErrUnsupported,
            ExiError::Grammar { .. } => RexiStatus::ErrGrammar,
            ExiError::Value { .. } => RexiStatus::ErrValue,
            ExiError::StringTable(_) => RexiStatus::ErrStringTable,
            ExiError::Schema(_) => RexiStatus::ErrSchema,
            ExiError::Internal(_) => RexiStatus::ErrInternal,
            ExiError::LimitExceeded { .. } => RexiStatus::ErrLimit,
            // An error kind added to rexi-core later.
            _ => RexiStatus::ErrInternal,
        };
        Fail::new(status, e.to_string())
    }
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_last_error(message: &str) {
    let c = CString::new(message.replace('\0', "\\0")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = c);
}

/// Runs `f` and turns its result into a status for C.
///
/// On success the last-error message is cleared. On failure the message is
/// stored and the failure's status is returned. A panic is caught here, so it
/// never unwinds into C code, and becomes `ErrPanic`. A handle that was in use
/// during a panic is not marked as failed, so the message tells the caller not
/// to use it again.
pub(crate) fn guard(f: impl FnOnce() -> Result<RexiStatus, Fail>) -> RexiStatus {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(status)) => {
            set_last_error("");
            status
        }
        Ok(Err(fail)) => {
            set_last_error(&fail.message);
            fail.status
        }
        Err(_) => {
            set_last_error("rexi panicked (a bug); the handle must not be used again");
            RexiStatus::ErrPanic
        }
    }
}

/// Borrows a NUL-terminated UTF-8 C string. Returns `ErrNullArgument` for NULL and
/// `ErrInvalidArgument` if the bytes are not UTF-8. `what` names the argument in
/// the error message.
///
/// # Safety
/// `p` is NULL, or points to a NUL-terminated string that stays valid for `'a`.
pub(crate) unsafe fn c_str<'a>(p: *const c_char, what: &str) -> Result<&'a str, Fail> {
    if p.is_null() {
        return Err(Fail::null(what));
    }
    // SAFETY: non-null and NUL-terminated per the caller's contract.
    let s = unsafe { CStr::from_ptr(p) };
    s.to_str()
        .map_err(|_| Fail::invalid(format!("{what} is not valid UTF-8")))
}

/// The message of the last failed `rexi_*` call on this thread, or `""` if the
/// last call worked. Never NULL. rexi owns the string: do not free it. It stays
/// valid until the next `rexi_*` call on the same thread. Any NUL bytes inside the
/// message are written as the two characters `\0`.
#[unsafe(no_mangle)]
pub extern "C" fn rexi_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

/// The library version, for example `"0.1.0"`. Never NULL. The string lives for
/// the whole program; do not free it.
#[unsafe(no_mangle)]
pub extern "C" fn rexi_version() -> *const c_char {
    static VERSION: &CStr =
        match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
            Ok(v) => v,
            Err(_) => c"unknown",
        };
    VERSION.as_ptr()
}
