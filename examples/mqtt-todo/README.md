# MQTT todo list

A shared todo list over MQTT, with every message in EXI. One Rust server holds
the list. Four command-line clients change it and read it: Rust, C, Java and C#.
Each client uses a different EXI library, so the example also checks that they
all agree.

| part | MQTT library | EXI library |
|---|---|---|
| `server/` (Rust) | [rumqttd](https://crates.io/crates/rumqttd) 0.20 (the broker, in the same process) + [rumqttc](https://crates.io/crates/rumqttc) 0.25 | rexi (`rexi-serde`, grammars from `rexi-schema` at start-up) |
| `rust-client/` | rumqttc 0.25 | rexi (`rexi-serde`) |
| `c-client/` | [Eclipse Paho MQTT C](https://github.com/eclipse-paho/paho.mqtt.c) v1.3.15, synchronous client | rexi's C API (`librexi.a`, `rexi.h`) |
| `java-client/` | [Eclipse Paho Java](https://github.com/eclipse-paho/paho.mqtt.java) `mqttv3` 1.2.5 | [EXIficient](https://github.com/EXIficient/exificient) 1.0.7 |
| `csharp-client/` | [MQTTnet](https://github.com/dotnet/MQTTnet) 5.2 | Nagasena for .NET (OpenEXI's C# port, svn r1188) |

`model/` holds the Rust message types, shared by the server and the Rust client.

None of this is built by the rexi workspace: the root `Cargo.toml` excludes
`examples/`, and the clients have their own build files.

## Running

```bash
./build.sh            # builds the server and every client whose toolchain is here
./demo.sh             # starts a server on a free port and runs all clients against it
```

`build.sh` needs `cargo`. Each client is built only when its tools are present,
and the script says which it skipped: the C client needs CMake, a C compiler and
git; the Java client Java 25 and Maven; the C# client the .NET 10 SDK (on `PATH`,
in `$DOTNET_ROOT` or in `~/.dotnet`) and Subversion. Paho C, Nagasena for .NET
and the Java Nagasena jars are pinned in `third-party.lock`.
`scripts/fetch-third-party.sh` checks them out into
`~/.cache/rexi/examples/mqtt-todo` (never into this tree); set
`REXI_EXAMPLES_THIRD_PARTY` to use another folder.

`demo.sh` has every client add an item, mark the next client's item done, and the
last client remove item 1. Then every client lists, and all lists must be the same.
It exits non-zero on any difference.

By hand:

```bash
target/release/todo-server --port 1883 &
target/release/todo-rust-client add "buy milk"
build/c-client/todo-c-client done 1
java -jar java-client/target/todo-java-client.jar add "walk the dog"
DOTNET_ROOT=~/.dotnet build/csharp-client/todo-csharp-client list
```

Every client takes `--host` and `--port` (default `127.0.0.1:1883`) and one of
`list`, `add TITLE`, `done ID` or `remove ID`, and prints the list the same way:

```text
revision 3
[x] 1 buy milk
[ ] 2 walk the dog
```

## Topics

| topic | who publishes | payload | retained |
|---|---|---|---|
| `todo/cmd` | clients | one `<command>` | no |
| `todo/list` | the server, after every command | the whole `<todoList>` | yes |

All messages use QoS 1 and MQTT 3.1.1. Because `todo/list` is
[retained](https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/os/mqtt-v3.1.1-os.html#_Toc398718038),
the broker sends the current list to a client as soon as it subscribes. So every
client does the same thing: subscribe to `todo/list` and take the retained list;
for `list`, print it; otherwise publish a command and print the next list. The
server publishes a list after every command, even one that changed nothing (an
unknown id), so a client always gets an answer; `revision` only goes up on a real
change.

## Messages

`todo.xsd`, namespace `urn:rexi:example:todo`:

```xml
<command from="c"><add title="buy milk"/></command>     <!-- or <done id="1"/>, <remove id="1"/> -->
<todoList revision="3">
  <item id="1" done="true">buy milk</item>
  <item id="2" done="false">walk the dog</item>
</todoList>
```

The types are kept simple: `xs:string`, `xs:unsignedInt` and `xs:boolean`.

## EXI settings

Every stream is **strict, schema-informed and bit-packed**, with **no options in
the header and no `$EXI` cookie**. The options are agreed out of band (everyone
reads this README), so the header is the single byte `0x80`
([EXI 5](https://www.w3.org/TR/exi/#header),
[EXI 5.4](https://www.w3.org/TR/exi/#options)). Strict mode leaves out everything
the schema does not allow (comments, unknown elements and so on), which keeps the
event codes as short as they can be
([EXI 8.5.4.4.2](https://www.w3.org/TR/exi/#addingProductionsStrict)).

How each library is told this:

- rexi (Rust): `ExiOptions::strict()` and `HeaderConfig::default()`.
- rexi (C): `rexi_options_strict()`.
- EXIficient: `FidelityOptions.createStrict()`, `CodingMode.BIT_PACKED`, default
  `EncodingOptions` (no cookie, no options).
- Nagasena for .NET: `GrammarOptions.STRICT_OPTIONS`, `AlignmentType.bitPacked`,
  `HeaderOptionsOutputType.none`, `OutputCookie = false`.

`model/src/lib.rs` has a test that rexi's bytes for a command and a list are the
same as EXIficient's.

### Nagasena for .NET and `todo.xsc`

The C# port of Nagasena can read compiled schemas but has no schema compiler. So
`csharp-client/todo.xsc` is `todo.xsd` compiled by Java Nagasena
(`EXISchema.writeOut`), and is committed. After changing `todo.xsd`, run
`scripts/gen-xsc.sh` (`--check` only tells you if the file is out of date). It
needs Java 25 and the Java Nagasena and Xerces jars from the W3C
[exi-testsuite](https://github.com/w3c/exi-testsuite). `scripts/fetch-third-party.sh`
checks out just that folder of it (`candidates/openexi/lib`). To use other
jars, set `$NAGASENA_LIB` to a folder holding `nagasena.jar` and
`nagasena-rta.jar`, and `$XERCES_JAR` to `xercesImpl.jar`. The C# port is built
from source with today's .NET SDK by `csharp-client/OpenEXI.Nagasena.csproj`, the
same project file the rexi-interop repository uses.

## Layout

| path | contents |
|---|---|
| `todo.xsd` | the schema every side uses |
| `Cargo.toml`, `model/`, `server/`, `rust-client/` | the Rust workspace |
| `c-client/` | CMake project; builds Paho C from the pinned checkout and links `librexi.a` |
| `java-client/` | Maven project; a shaded jar |
| `csharp-client/` | .NET projects (client + Nagasena for .NET) and `todo.xsc` |
| `third-party.lock`, `scripts/fetch-third-party.sh` | pins for Paho C, Nagasena for .NET and the Java Nagasena jars |
| `scripts/gen-xsc.sh`, `scripts/CompileXsc.java` | regenerate `todo.xsc` |
| `build.sh`, `demo.sh` | build, and run the end-to-end check |
