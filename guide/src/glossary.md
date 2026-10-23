# Words you'll see

This page explains the EXI words the guide uses, so you can follow every chapter.

## Schema

An XML Schema (`.xsd` file). It says which elements and attributes a document
may have, in what order, and what type each value is. rexi needs it on both ends.

## Grammar, grammar tables

The schema turned into rules a program can follow quickly: "after this, these
things may come next". rexi stores them as **grammar tables**. You build them
once with `rexi_schema::compile_files`, or ahead of time with `rexi-schema-gen`.

## Event

One piece of a document: the start of an element, an attribute, some text, or
the end of an element. EXI streams are a list of events.

## Strict (and non-strict)

**Strict** means the document follows the schema exactly. That lets EXI use the
fewest bits. **Non-strict** lets a document hold things the schema doesn't
declare, at a cost in size. rexi does strict only.

## Header

The first bits of every EXI stream. The smallest header is one byte. It can also
hold a cookie and the options.

## Options

Settings that say how the body was written, such as bit-packed or byte-aligned.
The decoder must know the same options the encoder used.

## Out of band

Agreed outside the stream, for example in your protocol's spec or a config file.
When the options are out of band, the stream leaves them out and stays smaller.

## Cookie

The four bytes `$EXI` at the very start of a stream. They are optional and help
tools spot an EXI file.

## Bit-packed, byte-aligned

Two ways to lay out the body. **Bit-packed** uses only the bits each item needs,
so it is the smallest. **Byte-aligned** starts each item on a whole byte, so it
is easier to read in a hex dump.

## String table

A list of strings the encoder and decoder both keep as they go. When a string
comes again, the stream sends a short number for it instead of the text.

## Text ("lexical") value

A value written as text, the way it looks in XML, such as `"42.95"`. EXI calls
this the lexical form. rexi can parse it with the schema's type for you.

## `no_std`

Rust code that runs without the standard library, for example on a
microcontroller with no operating system. `rexi-core` and `rexi-serde` work this
way and need only `alloc`.

## serde

The common Rust library for turning your own types into data formats and back.
`rexi-serde` uses it to turn your structs into EXI.
