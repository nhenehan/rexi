//! Values that cross the C boundary. Each kind matches one of the built-in EXI
//! datatype encodings ([EXI 7.1](https://www.w3.org/TR/exi/#encodingDatatypes)).

use std::ffi::c_char;

use rexi_core::{Float, Int, OwnedValue, Uint, Value};

use crate::status::{Fail, c_str};

/// The kind of a `RexiValue` (one of the `REXI_VALUE_*` constants).
pub type RexiValueKind = u32;

/// Input only: `lexical` is parsed with the datatype that the grammar calls for,
/// the same way text from an XML document would be. "Lexical" is the XML Schema
/// word for a value's text form.
pub const REXI_VALUE_LEXICAL: RexiValueKind = 0;
/// A string (`lexical`) ([EXI 7.1.10](https://www.w3.org/TR/exi/#encodingString)).
pub const REXI_VALUE_STRING: RexiValueKind = 1;
/// `bool_value` ([EXI 7.1.2](https://www.w3.org/TR/exi/#encodingBoolean)).
pub const REXI_VALUE_BOOLEAN: RexiValueKind = 2;
/// A signed integer that fits in `int_value`
/// ([EXI 7.1.5](https://www.w3.org/TR/exi/#encodingInteger)).
pub const REXI_VALUE_INTEGER: RexiValueKind = 3;
/// An unsigned integer that fits in `uint_value`
/// ([EXI 7.1.6](https://www.w3.org/TR/exi/#encodingUnsignedInteger)).
pub const REXI_VALUE_UNSIGNED: RexiValueKind = 4;
/// `double_value`, an EXI Float
/// ([EXI 7.1.4](https://www.w3.org/TR/exi/#encodingFloat)). On output, `lexical`
/// holds the exact value in its standard text form.
pub const REXI_VALUE_FLOAT: RexiValueKind = 5;
/// A decimal (`lexical`) ([EXI 7.1.3](https://www.w3.org/TR/exi/#encodingDecimal)).
pub const REXI_VALUE_DECIMAL: RexiValueKind = 6;
/// A date/time value (`lexical`)
/// ([EXI 7.1.8](https://www.w3.org/TR/exi/#encodingDateTime)).
pub const REXI_VALUE_DATETIME: RexiValueKind = 7;
/// Binary data (`bytes`, `bytes_len`)
/// ([EXI 7.1.1](https://www.w3.org/TR/exi/#encodingBinary)). On output, `lexical`
/// is hex or base64 text.
pub const REXI_VALUE_BINARY: RexiValueKind = 8;
/// An enumeration value ([EXI 7.2](https://www.w3.org/TR/exi/#encodingEnumerations)).
/// `uint_value` is its position (ordinal) in schema order, and `lexical` is the
/// value itself. On input the ordinal must fit in 32 bits.
pub const REXI_VALUE_ENUM: RexiValueKind = 9;
/// An `xs:list` value (`lexical`, items separated by single spaces)
/// ([EXI 7.1.11](https://www.w3.org/TR/exi/#encodingList)).
pub const REXI_VALUE_LIST: RexiValueKind = 10;
/// An integer too big for 64 bits (`lexical`).
pub const REXI_VALUE_BIG_INTEGER: RexiValueKind = 11;

/// A string slice owned by rexi. `ptr` points to `len` bytes of UTF-8 followed by a
/// NUL byte (not counted in `len`). It is valid only while the callback that
/// receives it runs. Copy it if you need it later; never free it.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RexiStr {
    /// Start of the UTF-8 bytes; a NUL byte follows the last one.
    pub ptr: *const c_char,
    /// Number of bytes, not counting the NUL.
    pub len: usize,
}

/// A value.
///
/// On output (decoder callbacks), `lexical` always holds the value's standard
/// text form, and the typed field that matches `kind` is also set. The other
/// fields are zero or NULL. rexi owns every pointer, and all of it is valid only
/// during the callback.
///
/// On input (encoder), only the field that matches `kind` is read:
/// - `lexical` for LEXICAL, STRING, DECIMAL, DATETIME, LIST and BIG_INTEGER. It
///   must be NUL-terminated UTF-8; `lexical_len` is not read. NULL returns
///   `ErrNullArgument`.
/// - `bytes` and `bytes_len` for BINARY. `bytes_len` is a count of bytes. `bytes`
///   may be NULL only when `bytes_len` is 0.
/// - the typed field for the other kinds.
///
/// rexi only reads the input during the call; the caller keeps ownership. Typed
/// values must match the grammar's datatype (INTEGER also fits decimals). Use
/// LEXICAL when unsure. An unknown `kind` returns `ErrInvalidArgument`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RexiValue {
    /// Which kind of value this is: one of the `REXI_VALUE_*` constants. It says
    /// which of the other fields is used.
    pub kind: RexiValueKind,
    /// The value of a BOOLEAN.
    pub bool_value: bool,
    /// The value of an INTEGER.
    pub int_value: i64,
    /// The value of an UNSIGNED, or the ordinal of an ENUM.
    pub uint_value: u64,
    /// The value of a FLOAT, as a 64-bit double.
    pub double_value: f64,
    /// The bytes of a BINARY value (base64Binary or hexBinary).
    pub bytes: *const u8,
    /// Number of bytes at `bytes`.
    pub bytes_len: usize,
    /// The value as NUL-terminated UTF-8 text. On output it is always set.
    pub lexical: *const c_char,
    /// Number of bytes at `lexical`, not counting the NUL (set on output only).
    pub lexical_len: usize,
}

/// A value made ready for the encoder. It borrows from the `RexiValue` it came
/// from.
pub(crate) enum Input<'a> {
    Lexical(&'a str),
    Owned(OwnedValue),
    Bytes(&'a [u8]),
}

impl Input<'_> {
    pub(crate) fn as_value(&self) -> Value<'_> {
        match self {
            Input::Lexical(s) => Value::Lexical(s),
            Input::Owned(v) => v.as_value(),
            Input::Bytes(b) => Value::Binary(b),
        }
    }
}

impl RexiValue {
    /// Reads the input form of the value, as described on [`RexiValue`].
    ///
    /// # Safety
    /// The pointer fields that `kind` uses are valid, as the struct's rules say.
    pub(crate) unsafe fn input(&self) -> Result<Input<'_>, Fail> {
        Ok(match self.kind {
            REXI_VALUE_LEXICAL
            | REXI_VALUE_STRING
            | REXI_VALUE_DECIMAL
            | REXI_VALUE_DATETIME
            | REXI_VALUE_LIST
            | REXI_VALUE_BIG_INTEGER => {
                // SAFETY: per the contract.
                Input::Lexical(unsafe { c_str(self.lexical, "value.lexical") }?)
            }
            REXI_VALUE_BOOLEAN => Input::Owned(OwnedValue::Boolean(self.bool_value)),
            REXI_VALUE_INTEGER => Input::Owned(OwnedValue::Integer(Int::Small(self.int_value))),
            REXI_VALUE_UNSIGNED => Input::Owned(OwnedValue::Unsigned(Uint::Small(self.uint_value))),
            REXI_VALUE_ENUM => {
                let ordinal = u32::try_from(self.uint_value)
                    .map_err(|_| Fail::invalid("enumeration ordinal out of range"))?;
                Input::Owned(OwnedValue::Enum(ordinal))
            }
            REXI_VALUE_FLOAT => {
                let d = self.double_value;
                let lexical = if d.is_nan() {
                    "NaN".to_string()
                } else if d.is_infinite() {
                    if d > 0.0 { "INF" } else { "-INF" }.to_string()
                } else {
                    // Rust prints the shortest text that reads back as the same
                    // double. `Float::from_lexical` then turns that text into EXI's
                    // mantissa and exponent (EXI 7.1.4).
                    d.to_string()
                };
                let f = Float::from_lexical(&lexical)
                    .ok_or_else(|| Fail::invalid(format!("float {d} not representable")))?;
                Input::Owned(OwnedValue::Float(f))
            }
            REXI_VALUE_BINARY => {
                if self.bytes_len == 0 {
                    Input::Bytes(&[])
                } else if self.bytes.is_null() {
                    return Err(Fail::null("value.bytes"));
                } else {
                    // SAFETY: `bytes` points to `bytes_len` readable bytes per the contract.
                    Input::Bytes(unsafe { std::slice::from_raw_parts(self.bytes, self.bytes_len) })
                }
            }
            other => return Err(Fail::invalid(format!("unknown value kind {other}"))),
        })
    }
}

/// A scratch buffer, reused for each event, that holds one NUL-terminated string
/// to hand to a callback. Each `set` overwrites the last string, so a `RexiStr`
/// from it is valid only until the next `set`.
#[derive(Default)]
pub(crate) struct Scratch(Vec<u8>);

impl Scratch {
    pub(crate) fn set(&mut self, s: &str) -> RexiStr {
        self.0.clear();
        self.0.extend_from_slice(s.as_bytes());
        self.0.push(0);
        RexiStr {
            ptr: self.0.as_ptr().cast(),
            len: s.len(),
        }
    }
}

/// Builds the output form of a decoded value. `lexical` must stay alive and
/// unchanged while the result is in use. For BINARY the result also points into
/// `value`, so `value` must stay alive too. Integers too big for 64 bits get kind
/// BIG_INTEGER and only `lexical`.
pub(crate) fn output(value: &OwnedValue, lexical: RexiStr) -> RexiValue {
    let mut v = RexiValue {
        kind: REXI_VALUE_STRING,
        bool_value: false,
        int_value: 0,
        uint_value: 0,
        double_value: 0.0,
        bytes: std::ptr::null(),
        bytes_len: 0,
        lexical: lexical.ptr,
        lexical_len: lexical.len,
    };
    match value {
        OwnedValue::String(_) => {}
        OwnedValue::Boolean(b) => {
            v.kind = REXI_VALUE_BOOLEAN;
            v.bool_value = *b;
        }
        OwnedValue::Integer(Int::Small(i)) => {
            v.kind = REXI_VALUE_INTEGER;
            v.int_value = *i;
        }
        OwnedValue::Unsigned(Uint::Small(u)) => {
            v.kind = REXI_VALUE_UNSIGNED;
            v.uint_value = *u;
        }
        OwnedValue::Integer(_) | OwnedValue::Unsigned(_) => v.kind = REXI_VALUE_BIG_INTEGER,
        OwnedValue::Float(f) => {
            v.kind = REXI_VALUE_FLOAT;
            v.double_value = match f.to_lexical().as_str() {
                "INF" => f64::INFINITY,
                "-INF" => f64::NEG_INFINITY,
                // The standard mantissa/exponent text, for example `441E-1`.
                // Rust's float parser reads it; if it fails, the result is NaN.
                lexical => lexical.parse().unwrap_or(f64::NAN),
            };
        }
        OwnedValue::Decimal(_) => v.kind = REXI_VALUE_DECIMAL,
        OwnedValue::DateTime(..) => v.kind = REXI_VALUE_DATETIME,
        OwnedValue::Binary(b) => {
            v.kind = REXI_VALUE_BINARY;
            v.bytes = b.as_ptr();
            v.bytes_len = b.len();
        }
        OwnedValue::Enum(o) => {
            v.kind = REXI_VALUE_ENUM;
            v.uint_value = u64::from(*o);
        }
        OwnedValue::List(_) => v.kind = REXI_VALUE_LIST,
        // A value kind added to rexi-core later: C still gets its text form.
        _ => v.kind = REXI_VALUE_STRING,
    }
    v
}
