/*
 * rexi C API smoke test.
 *
 * For each case in cases.tsv and each strict profile, decodes EXIficient's
 * reference stream through the callback API, checks the event dump against the
 * oracle's .events file, re-encodes every event through the encoder API, and
 * checks the bytes against the reference. Also exercises the static options
 * grammar, the error paths and the decoder limits. Exit status 0 means
 * everything matched.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "rexi.h"

static int failures = 0;

#define CHECK(cond, ...)                         \
    do {                                         \
        if (!(cond)) {                           \
            fprintf(stderr, "FAIL: " __VA_ARGS__); \
            fprintf(stderr, "\n");               \
            failures++;                          \
        }                                        \
    } while (0)

/* ---- small helpers ----------------------------------------------------- */

typedef struct {
    char *data;
    size_t len;
    size_t cap;
} Text;

static void text_append(Text *t, const char *s, size_t n) {
    if (t->len + n + 1 > t->cap) {
        size_t cap = t->cap ? t->cap : 256;
        while (cap < t->len + n + 1) cap *= 2;
        t->data = realloc(t->data, cap);
        if (!t->data) abort();
        t->cap = cap;
    }
    memcpy(t->data + t->len, s, n);
    t->len += n;
    t->data[t->len] = '\0';
}

static void text_puts(Text *t, const char *s) { text_append(t, s, strlen(s)); }

/* Same escaping as rexi_xml::events_dump. */
static void text_escaped(Text *t, const char *s, size_t n) {
    for (size_t i = 0; i < n; i++) {
        switch (s[i]) {
            case '\\': text_puts(t, "\\\\"); break;
            case '\n': text_puts(t, "\\n"); break;
            case '\r': text_puts(t, "\\r"); break;
            case '\t': text_puts(t, "\\t"); break;
            default: text_append(t, &s[i], 1);
        }
    }
}

static void text_qname(Text *t, const char *tag, RexiStr uri, RexiStr local) {
    text_puts(t, tag);
    text_puts(t, " {");
    text_append(t, uri.ptr, uri.len);
    text_puts(t, "}");
    text_append(t, local.ptr, local.len);
}

static unsigned char *read_file(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *buf = malloc((size_t)n + 1);
    if (!buf) abort();
    size_t got = fread(buf, 1, (size_t)n, f);
    fclose(f);
    buf[got] = '\0';
    *len = got;
    return buf;
}

/* Strips trailing newlines for comparison. */
static size_t trimmed(const char *s, size_t n) {
    while (n > 0 && (s[n - 1] == '\n' || s[n - 1] == '\r')) n--;
    return n;
}

/* ---- decode → dump + re-encode ----------------------------------------- */

typedef struct {
    Text dump;
    RexiEncoder *enc;
    RexiStatus enc_status; /* first encoder failure */
    char uri[512], local[512];
} Ctx;

static void note(Ctx *c, RexiStatus s) {
    if (s != REXI_STATUS_OK && c->enc_status == REXI_STATUS_OK) {
        c->enc_status = s;
        fprintf(stderr, "  encoder: %s\n", rexi_last_error());
    }
}

/* RexiStr is NUL-terminated, but copy to show the explicit length is enough. */
static const char *cstr(char *buf, size_t cap, RexiStr s) {
    size_t n = s.len < cap - 1 ? s.len : cap - 1;
    memcpy(buf, s.ptr, n);
    buf[n] = '\0';
    return buf;
}

static int on_sd(void *u) {
    Ctx *c = u;
    text_puts(&c->dump, "SD\n");
    note(c, rexi_start_document(c->enc));
    return 0;
}

static int on_ed(void *u) {
    Ctx *c = u;
    text_puts(&c->dump, "ED\n");
    note(c, rexi_end_document(c->enc));
    return 0;
}

static int on_se(RexiStr uri, RexiStr local, void *u) {
    Ctx *c = u;
    text_qname(&c->dump, "SE", uri, local);
    text_puts(&c->dump, "\n");
    note(c, rexi_start_element(c->enc, cstr(c->uri, sizeof c->uri, uri),
                               cstr(c->local, sizeof c->local, local)));
    return 0;
}

static int on_ee(void *u) {
    Ctx *c = u;
    text_puts(&c->dump, "EE\n");
    note(c, rexi_end_element(c->enc));
    return 0;
}

static int on_at(RexiStr uri, RexiStr local, const RexiValue *v, void *u) {
    Ctx *c = u;
    text_qname(&c->dump, "AT", uri, local);
    text_puts(&c->dump, " ");
    text_escaped(&c->dump, v->lexical, v->lexical_len);
    text_puts(&c->dump, "\n");
    /* Typed values go straight back in. */
    note(c, rexi_attribute(c->enc, cstr(c->uri, sizeof c->uri, uri),
                           cstr(c->local, sizeof c->local, local), v));
    return 0;
}

static int on_ch(const RexiValue *v, void *u) {
    Ctx *c = u;
    text_puts(&c->dump, "CH ");
    text_escaped(&c->dump, v->lexical, v->lexical_len);
    text_puts(&c->dump, "\n");
    note(c, rexi_characters(c->enc, v));
    return 0;
}

static int on_xsi_type(RexiStr uri, RexiStr local, void *u) {
    Ctx *c = u;
    text_qname(&c->dump, "XSI_TYPE", uri, local);
    text_puts(&c->dump, "\n");
    note(c, rexi_xsi_type(c->enc, cstr(c->uri, sizeof c->uri, uri),
                          cstr(c->local, sizeof c->local, local)));
    return 0;
}

static int on_xsi_nil(bool nil, void *u) {
    Ctx *c = u;
    text_puts(&c->dump, nil ? "XSI_NIL true\n" : "XSI_NIL false\n");
    note(c, rexi_xsi_nil(c->enc, nil));
    return 0;
}

static const RexiContentHandler HANDLER = {
    .start_document = on_sd,
    .end_document = on_ed,
    .start_element = on_se,
    .end_element = on_ee,
    .attribute = on_at,
    .characters = on_ch,
    .xsi_type = on_xsi_type,
    .xsi_nil = on_xsi_nil,
};

typedef struct {
    const char *name;
    bool byte_aligned;
    bool options_in_header;
} Profile;

static const Profile PROFILES[] = {
    {"strict-bp", false, false},
    {"strict-bp-opts", false, true},
    {"strict-ba", true, false},
};

static void run_case(const char *dir, const char *name, const char *main_xsd,
                     const char *imports) {
    char paths_buf[8][1024];
    const char *paths[8];
    size_t npaths = 0;
    snprintf(paths_buf[npaths], sizeof paths_buf[0], "%s/%s", dir, main_xsd);
    paths[npaths] = paths_buf[npaths];
    npaths++;
    if (imports && *imports) {
        char tmp[1024];
        snprintf(tmp, sizeof tmp, "%s", imports);
        for (char *tok = strtok(tmp, ","); tok && npaths < 8; tok = strtok(NULL, ",")) {
            snprintf(paths_buf[npaths], sizeof paths_buf[0], "%s/%s", dir, tok);
            paths[npaths] = paths_buf[npaths];
            npaths++;
        }
    }

    RexiTables *tables = NULL;
    RexiStatus s = rexi_tables_from_xsd(paths, npaths, &tables);
    CHECK(s == REXI_STATUS_OK, "%s: tables: %s", name, rexi_last_error());
    if (s != REXI_STATUS_OK) return;

    for (size_t p = 0; p < sizeof PROFILES / sizeof PROFILES[0]; p++) {
        const Profile *prof = &PROFILES[p];
        char path[1200];
        size_t ref_len = 0, ev_len = 0;
        snprintf(path, sizeof path, "%s/%s.%s.exi", dir, name, prof->name);
        unsigned char *ref = read_file(path, &ref_len);
        snprintf(path, sizeof path, "%s/%s.%s.events", dir, name, prof->name);
        char *events = (char *)read_file(path, &ev_len);
        CHECK(ref && events, "%s %s: missing reference files", name, prof->name);
        if (!ref || !events) {
            free(ref);
            free(events);
            continue;
        }

        RexiOptions opts = rexi_options_strict();
        opts.alignment = prof->byte_aligned ? REXI_ALIGN_BYTE_ALIGNED : REXI_ALIGN_BIT_PACKED;
        if (prof->options_in_header) {
            opts.include_options = true;
            opts.schema_id = name;
        }

        Ctx ctx;
        memset(&ctx, 0, sizeof ctx);
        ctx.enc_status = REXI_STATUS_OK;
        s = rexi_encoder_new(tables, &opts, &ctx.enc);
        CHECK(s == REXI_STATUS_OK, "%s %s: encoder: %s", name, prof->name, rexi_last_error());

        /* Out-of-band options only when the header does not carry them. */
        const RexiOptions *oob = prof->options_in_header ? NULL : &opts;
        RexiDecoder *dec = NULL;
        s = rexi_decoder_new(tables, ref, ref_len, oob, &dec);
        CHECK(s == REXI_STATUS_OK, "%s %s: decoder: %s", name, prof->name, rexi_last_error());
        size_t steps = 0;
        while (s == REXI_STATUS_OK) {
            s = rexi_decoder_next(dec, &HANDLER, &ctx);
            steps++;
        }
        CHECK(s == REXI_STATUS_END, "%s %s: decode: %d %s", name, prof->name, (int)s,
              rexi_last_error());
        /* After the end, next keeps reporting END. */
        CHECK(rexi_decoder_next(dec, &HANDLER, &ctx) == REXI_STATUS_END, "%s %s: END not sticky",
              name, prof->name);
        rexi_decoder_free(dec);

        size_t a = trimmed(ctx.dump.data ? ctx.dump.data : "", ctx.dump.len);
        size_t b = trimmed(events, ev_len);
        CHECK(a == b && memcmp(ctx.dump.data, events, a) == 0,
              "%s %s: event dump differs from oracle\n--- got ---\n%s\n--- want ---\n%s", name,
              prof->name, ctx.dump.data, events);

        RexiBuffer out = {0};
        s = rexi_encoder_finish(ctx.enc, &out);
        CHECK(s == REXI_STATUS_OK && ctx.enc_status == REXI_STATUS_OK,
              "%s %s: re-encode: %s", name, prof->name, rexi_last_error());
        CHECK(out.len == ref_len && memcmp(out.data, ref, ref_len) == 0,
              "%s %s: re-encoded %zu bytes differ from EXIficient's %zu", name, prof->name,
              out.len, ref_len);
        rexi_buffer_free(out);

        printf("ok %s %s (%zu events, %zu bytes)\n", name, prof->name, steps - 1, ref_len);
        free(ctx.dump.data);
        free(ref);
        free(events);
    }
    rexi_tables_free(tables);
}

/* ---- static options grammar and error paths ---------------------------- */

static int count_events(void *u) {
    (*(int *)u)++;
    return 0;
}
static int count_se(RexiStr uri, RexiStr local, void *u) {
    (void)uri;
    (void)local;
    (*(int *)u)++;
    return 0;
}
static int abort_on_se(RexiStr uri, RexiStr local, void *u) {
    (void)uri;
    (void)local;
    (void)u;
    return 7;
}

static void static_tables_and_errors(void) {
    static const char *EXI = "http://www.w3.org/2009/exi";
    const RexiTables *opt = rexi_tables_exi_options();
    RexiOptions o = rexi_options_strict();

    /* <header><common><schemaId>a</schemaId></common><strict/></header>
       must encode as 80 30 0D 84 (RFC 8428 / rexi-xml options_roundtrip). */
    RexiEncoder *enc = NULL;
    CHECK(rexi_encoder_new(opt, &o, &enc) == REXI_STATUS_OK, "options encoder");
    rexi_start_document(enc);
    rexi_start_element(enc, EXI, "header");
    rexi_start_element(enc, EXI, "common");
    rexi_start_element(enc, EXI, "schemaId");
    rexi_characters_lexical(enc, "a");
    rexi_end_element(enc);
    rexi_end_element(enc);
    rexi_start_element(enc, EXI, "strict");
    rexi_end_element(enc);
    rexi_end_element(enc);
    RexiStatus s = rexi_end_document(enc);
    CHECK(s == REXI_STATUS_OK, "options document: %s", rexi_last_error());
    RexiBuffer out = {0};
    CHECK(rexi_encoder_finish(enc, &out) == REXI_STATUS_OK, "finish: %s", rexi_last_error());
    static const unsigned char want[] = {0x80, 0x30, 0x0D, 0x84};
    CHECK(out.len == 4 && memcmp(out.data, want, 4) == 0, "options document bytes");

    int n = 0;
    RexiContentHandler h;
    memset(&h, 0, sizeof h);
    h.start_element = count_se;
    CHECK(rexi_decode(opt, out.data, out.len, &o, &h, &n) == REXI_STATUS_OK && n == 4,
          "decode options document: %d elements, %s", n, rexi_last_error());

    /* A callback returning nonzero aborts. */
    h.start_element = abort_on_se;
    s = rexi_decode(opt, out.data, out.len, &o, &h, &n);
    CHECK(s == REXI_STATUS_ERR_ABORTED && strstr(rexi_last_error(), "returned 7"),
          "abort: %d %s", (int)s, rexi_last_error());

    /* Truncated input. */
    memset(&h, 0, sizeof h);
    h.start_document = count_events;
    s = rexi_decode(opt, out.data, 2, &o, &h, &n);
    CHECK(s == REXI_STATUS_ERR_TRUNCATED && *rexi_last_error(), "truncated: %d %s", (int)s,
          rexi_last_error());
    rexi_buffer_free(out);

    /* Grammar violation, then the encoder is poisoned. */
    CHECK(rexi_encoder_new(opt, &o, &enc) == REXI_STATUS_OK, "encoder 2");
    rexi_start_document(enc);
    /* An unknown root would hit SE(*): unsupported (no built-in grammars). */
    s = rexi_start_element(enc, EXI, "bogus");
    CHECK(s == REXI_STATUS_ERR_UNSUPPORTED, "unknown root: %d %s", (int)s, rexi_last_error());
    rexi_encoder_free(enc);
    CHECK(rexi_encoder_new(opt, &o, &enc) == REXI_STATUS_OK, "encoder 3");
    rexi_start_document(enc);
    rexi_start_element(enc, EXI, "header");
    s = rexi_start_element(enc, EXI, "bogus");
    CHECK(s == REXI_STATUS_ERR_GRAMMAR, "grammar error: %d %s", (int)s, rexi_last_error());
    s = rexi_start_element(enc, EXI, "strict");
    CHECK(s == REXI_STATUS_ERR_STATE, "poisoned encoder: %d", (int)s);
    rexi_encoder_free(enc);

    /* Unsupported options and bad arguments. */
    RexiOptions nonstrict = rexi_options_strict();
    nonstrict.strict = false;
    CHECK(rexi_encoder_new(opt, &nonstrict, &enc) == REXI_STATUS_ERR_UNSUPPORTED,
          "non-strict: %s", rexi_last_error());
    CHECK(rexi_encoder_new(NULL, &o, &enc) == REXI_STATUS_ERR_NULL_ARGUMENT, "NULL tables");
    RexiTables *t = NULL;
    const char *missing[] = {"/nonexistent/schema.xsd"};
    CHECK(rexi_tables_from_xsd(missing, 1, &t) == REXI_STATUS_ERR_SCHEMA, "missing xsd");
    CHECK(rexi_tables_from_xsd_string("<not-a-schema/>", NULL, &t) == REXI_STATUS_ERR_SCHEMA,
          "bad xsd: %s", rexi_last_error());
    rexi_tables_free((RexiTables *)opt); /* no-op for the static handle */
    printf("ok static options grammar and error paths\n");
}

/* ---- decoder limits ---------------------------------------------------- */

static void decoder_limits(void) {
    RexiLimits d = rexi_limits_default();
    CHECK(d.max_depth == 256 && d.max_list_items == 1000000 && d.max_integer_bits == 4096 &&
              d.max_string_bytes == 268435456,
          "default limits");

    /* <a> may hold one more <a>, or nothing. */
    static const char *OPTIONAL =
        "<xs:schema xmlns:xs='http://www.w3.org/2001/XMLSchema'>"
        "<xs:element name='a'><xs:complexType><xs:sequence>"
        "<xs:element ref='a' minOccurs='0'/>"
        "</xs:sequence></xs:complexType></xs:element></xs:schema>";
    RexiTables *t = NULL;
    CHECK(rexi_tables_from_xsd_string(OPTIONAL, NULL, &t) == REXI_STATUS_OK, "tables: %s",
          rexi_last_error());
    RexiOptions o = rexi_options_strict();
    RexiEncoder *enc = NULL;
    CHECK(rexi_encoder_new(t, &o, &enc) == REXI_STATUS_OK, "encoder: %s", rexi_last_error());
    rexi_start_document(enc);
    rexi_start_element(enc, "", "a");
    rexi_start_element(enc, "", "a");
    rexi_end_element(enc);
    rexi_end_element(enc);
    CHECK(rexi_end_document(enc) == REXI_STATUS_OK, "<a><a/></a>: %s", rexi_last_error());
    RexiBuffer out = {0};
    CHECK(rexi_encoder_finish(enc, &out) == REXI_STATUS_OK, "finish: %s", rexi_last_error());

    /* Depth 2 is needed: a limit of 1 stops it, 2 lets it through. */
    RexiContentHandler h;
    memset(&h, 0, sizeof h);
    int n = 0;
    h.start_element = count_se;
    RexiLimits tiny = rexi_limits_default();
    tiny.max_depth = 1;
    RexiDecoder *dec = NULL;
    RexiStatus s = rexi_decoder_new_with_limits(t, out.data, out.len, &o, &tiny, &dec);
    CHECK(s == REXI_STATUS_OK, "decoder with limits: %s", rexi_last_error());
    while ((s = rexi_decoder_next(dec, &h, &n)) == REXI_STATUS_OK) {
    }
    CHECK(s == REXI_STATUS_ERR_LIMIT && strstr(rexi_last_error(), "nesting depth is over 1"),
          "depth 1: %d %s", (int)s, rexi_last_error());
    rexi_decoder_free(dec);

    tiny.max_depth = 2;
    n = 0;
    s = rexi_decode_with_limits(t, out.data, out.len, &o, &tiny, &h, &n);
    CHECK(s == REXI_STATUS_OK && n == 2, "depth 2: %d %s", (int)s, rexi_last_error());

    /* NULL limits are the defaults: a normal document goes through. */
    n = 0;
    s = rexi_decode_with_limits(t, out.data, out.len, &o, NULL, &h, &n);
    CHECK(s == REXI_STATUS_OK && n == 2, "NULL limits: %d %s", (int)s, rexi_last_error());
    rexi_buffer_free(out);
    rexi_tables_free(t);

    /* A required self-recursive <a>: two bytes open elements without end. With
       NULL limits (the defaults) and with the plain functions, it stops at 256. */
    static const char *REQUIRED =
        "<xs:schema xmlns:xs='http://www.w3.org/2001/XMLSchema'>"
        "<xs:element name='a'><xs:complexType><xs:sequence>"
        "<xs:element ref='a'/>"
        "</xs:sequence></xs:complexType></xs:element></xs:schema>";
    CHECK(rexi_tables_from_xsd_string(REQUIRED, NULL, &t) == REXI_STATUS_OK, "tables 2: %s",
          rexi_last_error());
    static const unsigned char evil[] = {0x80, 0x00};
    n = 0;
    s = rexi_decode_with_limits(t, evil, sizeof evil, &o, NULL, &h, &n);
    CHECK(s == REXI_STATUS_ERR_LIMIT && n == 256 &&
              strstr(rexi_last_error(), "nesting depth is over 256"),
          "recursion, NULL limits: %d %d %s", (int)s, n, rexi_last_error());
    n = 0;
    s = rexi_decode(t, evil, sizeof evil, &o, &h, &n);
    CHECK(s == REXI_STATUS_ERR_LIMIT && n == 256, "recursion, rexi_decode: %d %d %s", (int)s, n,
          rexi_last_error());
    s = rexi_decoder_new_with_limits(t, evil, sizeof evil, &o, &tiny, NULL);
    CHECK(s == REXI_STATUS_ERR_NULL_ARGUMENT, "NULL out: %d", (int)s);
    rexi_tables_free(t);
    printf("ok decoder limits\n");
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s <fixtures/interop>\n", argv[0]);
        return 2;
    }
    printf("rexi %s\n", rexi_version());
    char path[1024];
    snprintf(path, sizeof path, "%s/cases.tsv", argv[1]);
    size_t len = 0;
    char *tsv = (char *)read_file(path, &len);
    if (!tsv) {
        fprintf(stderr, "cannot read %s\n", path);
        return 2;
    }
    int cases = 0;
    char *save = NULL;
    for (char *line = strtok_r(tsv, "\n", &save); line; line = strtok_r(NULL, "\n", &save)) {
        if (strncmp(line, "name\t", 5) == 0 || !*line) continue;
        char *cols[4] = {0};
        int ncol = 0;
        char *p = line;
        while (ncol < 4) {
            cols[ncol++] = p;
            char *tab = strchr(p, '\t');
            if (!tab) break;
            *tab = '\0';
            p = tab + 1;
        }
        if (ncol < 3) continue;
        run_case(argv[1], cols[0], cols[2], ncol > 3 ? cols[3] : "");
        cases++;
    }
    free(tsv);
    CHECK(cases == 6, "expected 6 cases, found %d", cases);
    static_tables_and_errors();
    decoder_limits();
    if (failures) {
        fprintf(stderr, "%d failure(s)\n", failures);
        return 1;
    }
    printf("all ok\n");
    return 0;
}
