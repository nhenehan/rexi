# When things go wrong

In this chapter you'll learn what rexi's common errors mean and how to fix them.

rexi fails loudly instead of guessing. Most errors fall into a few groups:

| You see | Usually because | Fix |
|---|---|---|
| `Unsupported(..)` | You asked for an EXI feature rexi doesn't have, such as compression | Use [strict](glossary.md#strict-and-non-strict) options, bit-packed or byte-aligned |
| `Grammar { expected, got }` | An event came where the schema doesn't allow it | Check the order of elements, and put attributes first, sorted by name |
| `Value { datatype, reason }` | A value doesn't fit its type, such as `"two"` for an integer | Fix the value, or the schema |
| `Header("stream carries no options and none were given out of band")` | The stream has no options in its [header](glossary.md#header), and you passed `None` | Pass the same options the encoder used, or put them in the header |
| Wrong values, `Truncated`, or other odd errors | The two sides used different schemas or options | Use the same `.xsd` on both ends, and think about adding a `schemaId` |
| `Io(BufferFull)` | A `SliceSink` ran out of room | Use a bigger buffer |
| `Unsupported(RestrictedCharacterSet)` | A string type's `xs:pattern` allows fewer than 256 characters, such as `[A-Z]{2}[0-9]{4}`. EXI writes those values in a special compact form that rexi doesn't have yet. rexi refuses rather than write bytes other tools would misread | Leave that element out. Or, if you control both schemas, drop the pattern or use one with a large alphabet, such as `\w+` |
| `SchemaError::Refused { location, reason }` | An `xs:include` or `xs:import` names a file outside the main schema's folder, a URL, a pipe or device, or a file over 16 MiB | If you trust the schema set, set its top folder as the root; see [Untrusted schemas](untrusted-input.md#untrusted-schemas) |
| `SchemaError::TooLarge { max }` | One type's content would grow past 100,000 grammar rules: a huge `maxOccurs`, or groups that keep doubling | If you trust the schema, raise `max_grammar_size`; see [Grammar size](untrusted-input.md#grammar-size) |
| `Unsupported(Wildcard)` | The schema uses `xs:any` or `xs:anyAttribute` at that point, and the document uses it. rexi doesn't do wildcards yet | Declare the elements and attributes, if you control the schema |
| `LimitExceeded { limit, max }` | The stream asked for more than a decoder limit allows: too deep, a list too long, an integer too big, or too many bytes of strings. Either the stream is hostile or broken, or it is a very large real one | If you trust the stream, raise that limit; see [Untrusted input](untrusted-input.md) |

The error messages use the same words as the EXI spec, so you can look them up in
[EXI 1.0](https://www.w3.org/TR/exi/) when you need more detail.
