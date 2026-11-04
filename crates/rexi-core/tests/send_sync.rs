//! The public types can move to and be shared between threads
//! ([C-SEND-SYNC](https://rust-lang.github.io/api-guidelines/interoperability.html#c-send-sync)).
//! This only has to compile: a field that is not `Send` or `Sync` breaks the build.

use rexi_core::{
    DecodeLimits, Decoder, Encoder, Event, ExiError, ExiOptions, GrammarSet, GrammarSetBuf,
    OwnedEvent, OwnedValue, Value,
};

fn send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_and_sync() {
    send_sync::<Encoder<Vec<u8>, GrammarSetBuf>>();
    send_sync::<Encoder<Vec<u8>, &'static GrammarSet<'static>>>();
    send_sync::<Decoder<&'static [u8], &'static GrammarSetBuf>>();
    send_sync::<ExiError>();
    send_sync::<ExiOptions>();
    send_sync::<DecodeLimits>();
    send_sync::<GrammarSetBuf>();
    send_sync::<GrammarSet<'static>>();
    send_sync::<Event<'static>>();
    send_sync::<OwnedEvent>();
    send_sync::<Value<'static>>();
    send_sync::<OwnedValue>();
}
