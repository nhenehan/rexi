import java.io.DataOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import org.openexi.schema.EXISchema;
import org.openexi.scomp.EXISchemaFactory;
import org.xml.sax.InputSource;

/**
 * Compiles an XML Schema with Java Nagasena and writes it in Nagasena's binary
 * schema form (EXISchema.writeOut). Nagasena for .NET can read that form
 * (EXISchema.readIn) but has no schema compiler of its own.
 *
 *   java -cp NAGASENA_JARS scripts/CompileXsc.java todo.xsd todo.xsc
 */
public final class CompileXsc {
    public static void main(String[] args) throws Exception {
        EXISchema schema = new EXISchemaFactory().compile(new InputSource(new File(args[0]).toURI().toString()));
        try (DataOutputStream out = new DataOutputStream(new FileOutputStream(args[1]))) {
            schema.writeOut(out);
        }
    }
}
