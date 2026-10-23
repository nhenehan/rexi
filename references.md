# REXI References

This file is for research and design references only.

## Standards and Specifications

1. EXI Format 1.0 (W3C): https://www.w3.org/TR/exi/
2. EXI Primer (W3C): https://www.w3.org/TR/exi-primer/
3. XML Test Suite (W3C): https://www.w3.org/XML/Test/
4. RFC 8428 (Sensor Measurement Lists): https://www.rfc-editor.org/rfc/rfc8428

## Official and Practical Test Sources

1. W3C EXI testsuite: https://github.com/w3c/exi-testsuite

## Reference Implementations

1. EXIficient (Java): https://github.com/EXIficient/exificient
2. nhenehan/exip (C): https://github.com/nhenehan/exip
3. exip historical upstream (SVN): https://sourceforge.net/p/exip/code
4. OpenEXI Nagasena (SVN): https://svn.code.sf.net/p/openexi/code

## XML and Parser Tooling References

1. Xerces-C: https://xerces.apache.org/xerces-c/
2. libxml2: https://github.com/gnome/libxml2

## Platform and Toolchain References

1. Contiki OS: https://www.contiki-os.org/
2. RIOT OS: https://www.riot-os.org/
3. MinGW-w64: https://www.mingw-w64.org/

## Native Testing Framework References

1. CMocka: https://cmocka.org/
2. Unity (ThrowTheSwitch): https://github.com/ThrowTheSwitch/unity
3. Greatest: https://github.com/silentbicycle/greatest

## Reference Usage Intent (Design Only)

When behavior interpretation needs external comparison:

1. EXIficient is the primary behavioral reference.
2. nhenehan/exip is the second reference, for C.
3. Other references are supporting context for diagnostics and design research.
