//! C ABI for rexi, the strict schema-informed EXI codec. ("Schema-informed" means
//! the grammars are built from an XML Schema, not learned while reading, see
//! [EXI 8.5](https://www.w3.org/TR/exi/#informedGrammars); "strict" means the stream
//! may only hold what the schema allows, see
//! [EXI 5.4](https://www.w3.org/TR/exi/#options).) The C header is
//! `include/rexi.h`. cbindgen writes it at build time.
//!
//! The API is shaped like exip's. The encoder is a handle with one call per EXI
//! event (`rexi_start_element`, `rexi_attribute`, `rexi_characters`, ...). The
//! decoder hands events to a struct of callbacks (`RexiContentHandler`). It hands
//! over one event per `rexi_decoder_next` call, or all of them with `rexi_decode`.
//! The events are the ones listed in [EXI 4](https://www.w3.org/TR/exi/#streams).
//!
//! Rules that hold for the whole API:
//! - Every call that can fail returns a `RexiStatus`. `rexi_last_error()` holds the
//!   error message. Each thread has its own message.
//! - Input strings are NUL-terminated UTF-8. Output strings (`RexiStr`) are also
//!   NUL-terminated, carry their length, and are valid only while the callback
//!   that got them is running.
//! - Grammar tables (`RexiTables`) come from XSD files (`rexi_tables_from_xsd`) or
//!   from a static table that `rexi-schema-gen` wrote (`RexiTables::from_static`
//!   on the Rust side). Each encoder and decoder keeps its own reference to them.
//! - The decoder has limits (`RexiLimits`) on nesting depth, list length,
//!   integer size and total string bytes, so a hostile stream cannot use up time
//!   or memory. `rexi_decoder_new` uses safe defaults; pick others with
//!   `rexi_decoder_new_with_limits`. Going over one gives `REXI_STATUS_ERR_LIMIT`.
//! - After an error, an encoder or decoder only returns `REXI_STATUS_ERR_STATE`.
//! - A Rust panic never crosses into C. It turns into `REXI_STATUS_ERR_PANIC`.
//!
//! This is the only crate in the workspace that is allowed to use `unsafe`.

#![warn(missing_docs)]

mod decoder;
mod encoder;
mod limits;
mod options;
mod status;
mod tables;
mod value;

pub use decoder::{
    RexiContentHandler, RexiDecoder, rexi_decode, rexi_decode_with_limits, rexi_decoder_free,
    rexi_decoder_new, rexi_decoder_new_with_limits, rexi_decoder_next,
};
pub use encoder::{
    RexiBuffer, RexiEncoder, rexi_attribute, rexi_attribute_lexical, rexi_buffer_free,
    rexi_characters, rexi_characters_lexical, rexi_encoder_finish, rexi_encoder_free,
    rexi_encoder_new, rexi_end_document, rexi_end_element, rexi_start_document, rexi_start_element,
    rexi_xsi_nil, rexi_xsi_type,
};
pub use limits::{RexiLimits, rexi_limits_default};
pub use options::{
    REXI_ALIGN_BIT_PACKED, REXI_ALIGN_BYTE_ALIGNED, RexiOptions, rexi_options_strict,
};
pub use status::{RexiStatus, rexi_last_error, rexi_version};
pub use tables::{
    RexiTables, rexi_tables_exi_options, rexi_tables_free, rexi_tables_from_xsd,
    rexi_tables_from_xsd_string,
};
pub use value::*;
