package example.todo;

import com.siemens.ct.exi.core.CodingMode;
import com.siemens.ct.exi.core.EXIFactory;
import com.siemens.ct.exi.core.EncodingOptions;
import com.siemens.ct.exi.core.FidelityOptions;
import com.siemens.ct.exi.core.helpers.DefaultEXIFactory;
import com.siemens.ct.exi.grammars.GrammarFactory;
import com.siemens.ct.exi.main.api.sax.EXIResult;
import com.siemens.ct.exi.main.api.sax.SAXFactory;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.StringReader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import javax.xml.parsers.SAXParserFactory;
import org.eclipse.paho.client.mqttv3.MqttClient;
import org.eclipse.paho.client.mqttv3.MqttConnectOptions;
import org.eclipse.paho.client.mqttv3.persist.MemoryPersistence;
import org.xml.sax.Attributes;
import org.xml.sax.InputSource;
import org.xml.sax.XMLReader;
import org.xml.sax.helpers.DefaultHandler;

/**
 * The todo list from the command line, in Java: Eclipse Paho for MQTT 3.1.1 and
 * EXIficient for EXI.
 *
 * <pre>
 *   java -jar todo-java-client.jar [--host H] [--port N] list | add TITLE | done ID | remove ID
 * </pre>
 *
 * It works like every other client: subscribe to todo/list (the broker sends the
 * retained list at once), print it for "list", or else publish one command to
 * todo/cmd and print the next list the server sends.
 *
 * <p>EXI: strict, schema-informed, bit-packed, no options in the header and no
 * cookie. Both sides agree on that up front ("out of band", see
 * <a href="https://www.w3.org/TR/exi/#options">EXI 5.4</a>). EXIficient's
 * defaults already give a bare header; only strict has to be switched on.
 */
public final class TodoClient {
    static final String NS = "urn:rexi:example:todo";
    static final String CMD_TOPIC = "todo/cmd";
    static final String LIST_TOPIC = "todo/list";
    static final long TIMEOUT_S = 10;

    private TodoClient() {}

    record Item(long id, boolean done, String title) {}

    record TodoList(long revision, List<Item> items) {
        /** The same text every client prints, so the demo can compare them. */
        String format() {
            StringBuilder b = new StringBuilder("revision ").append(revision).append('\n');
            for (Item i : items) {
                b.append('[').append(i.done ? 'x' : ' ').append("] ").append(i.id).append(' ').append(i.title)
                        .append('\n');
            }
            return b.toString();
        }
    }

    public static void main(String[] argv) {
        try {
            System.exit(run(argv));
        } catch (Exception e) {
            System.err.println("todo-java-client: " + e);
            System.exit(1);
        }
    }

    static int run(String[] argv) throws Exception {
        String host = "127.0.0.1";
        int port = 1883;
        List<String> rest = new ArrayList<>();
        for (int i = 0; i < argv.length; i++) {
            switch (argv[i]) {
                case "--host" -> host = argv[++i];
                case "--port" -> port = Integer.parseInt(argv[++i]);
                default -> rest.add(argv[i]);
            }
        }
        String action = null;
        if (rest.isEmpty()) {
            return usage();
        }
        switch (rest.get(0)) {
            case "list" -> action = null;
            case "add" -> action = rest.size() == 2 ? "<add title=\"" + escape(rest.get(1)) + "\"/>" : null;
            case "done", "remove" -> action = rest.size() == 2
                    ? "<" + rest.get(0) + " id=\"" + Long.parseUnsignedLong(rest.get(1)) + "\"/>" : null;
            default -> {
                return usage();
            }
        }
        if (action == null && !rest.get(0).equals("list")) {
            return usage();
        }

        EXIFactory exi = exiFactory();
        BlockingQueue<byte[]> lists = new LinkedBlockingQueue<>();
        MqttClient mqtt = new MqttClient("tcp://" + host + ":" + port, "todo-java-" + ProcessHandle.current().pid(),
                new MemoryPersistence());
        MqttConnectOptions options = new MqttConnectOptions();
        options.setMqttVersion(MqttConnectOptions.MQTT_VERSION_3_1_1);
        options.setCleanSession(true);
        mqtt.connect(options);
        try {
            // The broker answers the subscription with the retained list.
            mqtt.subscribe(LIST_TOPIC, 1, (topic, message) -> lists.add(message.getPayload()));
            byte[] first = next(lists);
            if (action == null) {
                System.out.print(decode(exi, first).format());
                return 0;
            }
            String xml = "<command xmlns=\"" + NS + "\" from=\"java\">" + action + "</command>";
            mqtt.publish(CMD_TOPIC, encode(exi, xml), 1, false);
            System.out.print(decode(exi, next(lists)).format());
            return 0;
        } finally {
            mqtt.disconnect();
            mqtt.close();
        }
    }

    private static int usage() {
        System.err.println("usage: todo-java-client [--host H] [--port N] list | add TITLE | done ID | remove ID");
        return 2;
    }

    private static byte[] next(BlockingQueue<byte[]> q) throws InterruptedException {
        byte[] b = q.poll(TIMEOUT_S, TimeUnit.SECONDS);
        if (b == null) {
            throw new IllegalStateException("timed out waiting for " + LIST_TOPIC);
        }
        return b;
    }

    /** Strict, bit-packed, bare header; grammars from todo.xsd (a resource). */
    static EXIFactory exiFactory() throws Exception {
        Path xsd = Files.createTempFile("todo", ".xsd");
        try (InputStream in = TodoClient.class.getResourceAsStream("/todo.xsd")) {
            Files.copy(in, xsd, StandardCopyOption.REPLACE_EXISTING);
        }
        try {
            EXIFactory f = DefaultEXIFactory.newInstance();
            f.setGrammars(GrammarFactory.newInstance().createGrammars(xsd.toString()));
            f.setFidelityOptions(FidelityOptions.createStrict());
            f.setCodingMode(CodingMode.BIT_PACKED);
            f.setEncodingOptions(EncodingOptions.createDefault());
            return f;
        } finally {
            Files.deleteIfExists(xsd);
        }
    }

    /** XML text in, EXI out: EXIficient takes SAX events from a normal parser. */
    static byte[] encode(EXIFactory f, String xml) throws Exception {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        EXIResult result = new EXIResult(f);
        result.setOutputStream(out);
        SAXParserFactory spf = SAXParserFactory.newDefaultInstance();
        spf.setNamespaceAware(true);
        XMLReader r = spf.newSAXParser().getXMLReader();
        r.setContentHandler(result.getHandler());
        r.parse(new InputSource(new StringReader(xml)));
        return out.toByteArray();
    }

    /** EXI in, list out: EXIficient's reader sends SAX events to a handler. */
    static TodoList decode(EXIFactory f, byte[] exi) throws Exception {
        XMLReader r = new SAXFactory(f).createEXIReader();
        long[] revision = {-1};
        List<Item> items = new ArrayList<>();
        r.setContentHandler(new DefaultHandler() {
            private final StringBuilder text = new StringBuilder();
            private long id;
            private boolean done;

            @Override
            public void startElement(String uri, String local, String qName, Attributes atts) {
                if (local.equals("todoList")) {
                    revision[0] = Long.parseLong(atts.getValue("revision"));
                } else if (local.equals("item")) {
                    id = Long.parseLong(atts.getValue("id"));
                    String d = atts.getValue("done");
                    done = d.equals("true") || d.equals("1");
                    text.setLength(0);
                }
            }

            @Override
            public void characters(char[] ch, int start, int length) {
                text.append(ch, start, length);
            }

            @Override
            public void endElement(String uri, String local, String qName) {
                if (local.equals("item")) {
                    items.add(new Item(id, done, text.toString()));
                }
            }
        });
        r.parse(new InputSource(new ByteArrayInputStream(exi)));
        return new TodoList(revision[0], items);
    }

    static String escape(String s) {
        return s.replace("&", "&amp;").replace("<", "&lt;").replace("\"", "&quot;");
    }
}
