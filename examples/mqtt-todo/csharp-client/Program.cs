// todo-csharp-client: the todo list from the command line, in C#.
//
//   todo-csharp-client [--host H] [--port N] list | add TITLE | done ID | remove ID
//
// MQTT: MQTTnet, MQTT 3.1.1. EXI: Nagasena for .NET (OpenEXI's C# port).
//
// It works like every other client: subscribe to todo/list (the broker sends the
// retained list at once), print it for "list", or else publish one command to
// todo/cmd and print the next list the server sends.
//
// EXI streams are strict, schema-informed and bit-packed, with a bare header (no
// options, no cookie): both sides know the options up front
// (https://www.w3.org/TR/exi/#options). Nagasena for .NET cannot compile a
// schema, so it reads todo.xsc, which Java Nagasena compiled from todo.xsd.

using System;
using System.Buffers;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Security;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using MQTTnet;
using MQTTnet.Formatter;
using MQTTnet.Protocol;
using Nagasena.Proc;
using Nagasena.Proc.Common;
using Nagasena.Proc.Grammars;
using Nagasena.Proc.IO;
using Nagasena.Sax;
using Nagasena.Schema;
using Org.System.Xml.Sax;

internal static class TodoClient {
    private const string Ns = "urn:rexi:example:todo";
    private const string CmdTopic = "todo/cmd";
    private const string ListTopic = "todo/list";
    private static readonly TimeSpan Timeout = TimeSpan.FromSeconds(10);

    private static async Task<int> Main(string[] argv) {
        try {
            return await Run(argv);
        } catch (Exception e) {
            Console.Error.WriteLine($"todo-csharp-client: {e.GetType().Name}: {e.Message}");
            return 1;
        }
    }

    private static int Usage() {
        Console.Error.WriteLine("usage: todo-csharp-client [--host H] [--port N] list | add TITLE | done ID | remove ID");
        return 2;
    }

    private static async Task<int> Run(string[] argv) {
        string host = "127.0.0.1";
        int port = 1883;
        var rest = new List<string>();
        for (int i = 0; i < argv.Length; i++) {
            if (argv[i] == "--host" && i + 1 < argv.Length) {
                host = argv[++i];
            } else if (argv[i] == "--port" && i + 1 < argv.Length) {
                port = int.Parse(argv[++i], CultureInfo.InvariantCulture);
            } else {
                rest.Add(argv[i]);
            }
        }
        string? action = (rest.Count, rest.Count > 0 ? rest[0] : "") switch {
            (1, "list") => null,
            (2, "add") => $"<add title=\"{SecurityElement.Escape(rest[1])}\"/>",
            (2, "done" or "remove") => $"<{rest[0]} id=\"{uint.Parse(rest[1], CultureInfo.InvariantCulture)}\"/>",
            _ => "?",
        };
        if (action == "?") {
            return Usage();
        }

        GrammarCache grammars;
        using (var xsc = File.OpenRead(Path.Combine(AppContext.BaseDirectory, "todo.xsc"))) {
            grammars = new GrammarCache(EXISchema.readIn(xsc), GrammarOptions.STRICT_OPTIONS);
        }

        // Messages on todo/list, in the order they arrive.
        var lists = System.Threading.Channels.Channel.CreateUnbounded<byte[]>();
        using var client = new MqttClientFactory().CreateMqttClient();
        client.ApplicationMessageReceivedAsync += e => {
            if (e.ApplicationMessage.Topic == ListTopic) {
                lists.Writer.TryWrite(e.ApplicationMessage.Payload.ToArray());
            }
            return Task.CompletedTask;
        };
        var options = new MqttClientOptionsBuilder()
            .WithTcpServer(host, port)
            .WithClientId($"todo-csharp-{Environment.ProcessId}")
            .WithProtocolVersion(MqttProtocolVersion.V311)
            .WithCleanSession()
            .Build();
        using var cts = new CancellationTokenSource(Timeout);
        await client.ConnectAsync(options, cts.Token);
        // The broker answers the subscription with the retained list.
        await client.SubscribeAsync(new MqttClientSubscribeOptionsBuilder()
            .WithTopicFilter(f => f.WithTopic(ListTopic).WithQualityOfServiceLevel(MqttQualityOfServiceLevel.AtLeastOnce))
            .Build(), cts.Token);
        byte[] list = await lists.Reader.ReadAsync(cts.Token);
        if (action != null) {
            byte[] cmd = Encode(grammars, $"<command xmlns=\"{Ns}\" from=\"csharp\">{action}</command>");
            await client.PublishAsync(new MqttApplicationMessageBuilder()
                .WithTopic(CmdTopic)
                .WithPayload(cmd)
                .WithQualityOfServiceLevel(MqttQualityOfServiceLevel.AtLeastOnce)
                .Build(), cts.Token);
            list = await lists.Reader.ReadAsync(cts.Token);
        }
        Console.Out.Write(Format(grammars, list));
        await client.DisconnectAsync();
        return 0;
    }

    // XML text in, EXI out. The Transmogrifier reads the XML with .NET's XmlReader.
    private static byte[] Encode(GrammarCache grammars, string xml) {
        var t = new Transmogrifier {
            GrammarCache = grammars,
            AlignmentType = AlignmentType.bitPacked,
            OutputOptions = HeaderOptionsOutputType.none,
            OutputCookie = false,
        };
        var output = new MemoryStream();
        t.OutputStream = output;
        t.encode(new InputSource<Stream>(new MemoryStream(Encoding.UTF8.GetBytes(xml))));
        return output.ToArray();
    }

    // EXI in, the same text every client prints. The decoder hands out one event
    // at a time; attribute values come as text.
    private static string Format(GrammarCache grammars, byte[] exi) {
        var d = new EXIDecoder { GrammarCache = grammars, AlignmentType = AlignmentType.bitPacked };
        d.InputStream = new MemoryStream(exi, false);
        Scanner scanner = d.processHeader();
        var b = new StringBuilder();
        bool inItem = false;
        string id = "", title = "";
        bool done = false;
        EventDescription ev;
        while ((ev = scanner.nextEvent()) != null) {
            switch (ev.EventKind) {
                case EventDescription_Fields.EVENT_SE when ev.Name == "item":
                    inItem = true;
                    id = title = "";
                    done = false;
                    break;
                case EventDescription_Fields.EVENT_AT:
                    string value = ev.Characters.makeString();
                    switch (ev.Name) {
                        case "revision":
                            b.Append("revision ").Append(value).Append('\n');
                            break;
                        case "id":
                            id = value;
                            break;
                        case "done":
                            done = value == "true" || value == "1";
                            break;
                    }
                    break;
                case EventDescription_Fields.EVENT_CH when inItem:
                    title += ev.Characters.makeString();
                    break;
                case EventDescription_Fields.EVENT_EE when inItem:
                    b.Append('[').Append(done ? 'x' : ' ').Append("] ").Append(id).Append(' ').Append(title).Append('\n');
                    inItem = false;
                    break;
            }
        }
        return b.ToString();
    }
}
