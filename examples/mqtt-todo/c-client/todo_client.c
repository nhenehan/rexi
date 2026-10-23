/*
 * todo-c-client: the todo list from the command line, in C.
 *
 *   todo-c-client [--host H] [--port N] [--xsd todo.xsd] list
 *   todo-c-client [--host H] [--port N] [--xsd todo.xsd] add TITLE | done ID | remove ID
 *
 * MQTT: Eclipse Paho C, synchronous client (MQTTClient.h), MQTT 3.1.1.
 * EXI:  rexi's C API (rexi.h). The grammars are compiled from todo.xsd at start-up.
 *
 * It works like every other client: subscribe to todo/list (the broker sends the
 * retained list at once), print it for "list", or else publish one command to
 * todo/cmd and print the next list the server sends.
 *
 * EXI streams are strict, schema-informed and bit-packed, with a bare header (no
 * options, no cookie): both sides know the options up front
 * (https://www.w3.org/TR/exi/#options). rexi_options_strict() is exactly that.
 */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "MQTTClient.h"
#include "rexi.h"

#define NS "urn:rexi:example:todo"
#define CMD_TOPIC "todo/cmd"
#define LIST_TOPIC "todo/list"
#define TIMEOUT_MS 10000L

static const char *g_host = "127.0.0.1";
static int g_port = 1883;
static const char *g_xsd = TODO_XSD;

/* ---- the list, as decoded ------------------------------------------------ */

typedef struct {
    unsigned long id;
    bool done;
    char *title;
} Item;

typedef struct {
    unsigned long revision;
    Item *items;
    size_t len, cap;
    bool in_item; /* inside <item>: text is the title */
} TodoList;

static char *copy_str(const char *p, size_t n) {
    char *s = malloc(n + 1);
    if (s) {
        memcpy(s, p, n);
        s[n] = '\0';
    }
    return s;
}

static bool is(RexiStr s, const char *lit) {
    return s.len == strlen(lit) && memcmp(s.ptr, lit, s.len) == 0;
}

/* rexi calls these while it decodes; each returns 0 to keep going. */

static int on_start_element(RexiStr uri, RexiStr local, void *user) {
    TodoList *l = user;
    (void)uri;
    if (is(local, "item")) {
        if (l->len == l->cap) {
            size_t cap = l->cap ? l->cap * 2 : 8;
            Item *grown = realloc(l->items, cap * sizeof *grown);
            if (!grown)
                return 1;
            l->items = grown;
            l->cap = cap;
        }
        l->items[l->len++] = (Item){0, false, NULL};
        l->in_item = true;
    }
    return 0;
}

static int on_end_element(void *user) {
    ((TodoList *)user)->in_item = false;
    return 0;
}

static int on_attribute(RexiStr uri, RexiStr local, const RexiValue *v, void *user) {
    TodoList *l = user;
    (void)uri;
    /* Typed values arrive typed: unsignedInt as uint_value, boolean as bool_value. */
    if (is(local, "revision"))
        l->revision = (unsigned long)v->uint_value;
    else if (is(local, "id") && l->len)
        l->items[l->len - 1].id = (unsigned long)v->uint_value;
    else if (is(local, "done") && l->len)
        l->items[l->len - 1].done = v->bool_value;
    return 0;
}

static int on_characters(const RexiValue *v, void *user) {
    TodoList *l = user;
    if (l->in_item && l->len) {
        Item *it = &l->items[l->len - 1];
        free(it->title);
        it->title = copy_str(v->lexical, v->lexical_len);
        if (!it->title)
            return 1;
    }
    return 0;
}

static void list_free(TodoList *l) {
    for (size_t i = 0; i < l->len; i++)
        free(l->items[i].title);
    free(l->items);
}

static int decode_list(const RexiTables *t, const void *data, size_t len, TodoList *out) {
    RexiContentHandler h = {0};
    h.start_element = on_start_element;
    h.end_element = on_end_element;
    h.attribute = on_attribute;
    h.characters = on_characters;
    RexiOptions o = rexi_options_strict(); /* out of band: the header has none */
    memset(out, 0, sizeof *out);
    if (rexi_decode(t, data, len, &o, &h, out) != REXI_STATUS_OK) {
        fprintf(stderr, "todo-c-client: bad list: %s\n", rexi_last_error());
        return -1;
    }
    return 0;
}

/* The same text every client prints, so the demo can compare them. */
static void print_list(const TodoList *l) {
    printf("revision %lu\n", l->revision);
    for (size_t i = 0; i < l->len; i++)
        printf("[%c] %lu %s\n", l->items[i].done ? 'x' : ' ', l->items[i].id,
               l->items[i].title ? l->items[i].title : "");
}

/* ---- the command, encoded event by event --------------------------------- */

/*
 * <command from="c"><add title="..."/></command>, or <done id="N"/> / <remove id="N"/>.
 * In strict mode the attributes come right after their start tag, in schema order
 * (https://www.w3.org/TR/exi/#addingProductionsStrict).
 */
static int encode_command(const RexiTables *t, const char *verb, const char *arg,
                          RexiBuffer *out) {
    RexiOptions o = rexi_options_strict();
    RexiEncoder *e = NULL;
    if (rexi_encoder_new(t, &o, &e) != REXI_STATUS_OK)
        goto fail;
    if (rexi_start_document(e) || rexi_start_element(e, NS, "command") ||
        rexi_attribute_lexical(e, "", "from", "c") || rexi_start_element(e, NS, verb))
        goto fail_enc;
    if (strcmp(verb, "add") == 0) {
        if (rexi_attribute_lexical(e, "", "title", arg))
            goto fail_enc;
    } else {
        char *end = NULL;
        unsigned long id = strtoul(arg, &end, 10);
        if (*arg == '\0' || *end != '\0') {
            fprintf(stderr, "todo-c-client: bad id %s\n", arg);
            rexi_encoder_free(e);
            return -1;
        }
        RexiValue v = {0};
        v.kind = REXI_VALUE_UNSIGNED;
        v.uint_value = id;
        if (rexi_attribute(e, "", "id", &v))
            goto fail_enc;
    }
    if (rexi_end_element(e) || rexi_end_element(e) || rexi_end_document(e))
        goto fail_enc;
    /* finish frees the encoder whether it works or not */
    if (rexi_encoder_finish(e, out) != REXI_STATUS_OK)
        goto fail;
    return 0;
fail_enc:
    rexi_encoder_free(e);
fail:
    fprintf(stderr, "todo-c-client: encoding failed: %s\n", rexi_last_error());
    return -1;
}

/* ---- MQTT ---------------------------------------------------------------- */

/* Waits for the next message on todo/list and decodes it. */
static int next_list(MQTTClient c, const RexiTables *t, TodoList *out) {
    for (;;) {
        char *topic = NULL;
        int topic_len = 0;
        MQTTClient_message *m = NULL;
        int rc = MQTTClient_receive(c, &topic, &topic_len, &m, TIMEOUT_MS);
        if (rc != MQTTCLIENT_SUCCESS && rc != MQTTCLIENT_TOPICNAME_TRUNCATED) {
            fprintf(stderr, "todo-c-client: receive failed (%d)\n", rc);
            return -1;
        }
        if (m == NULL) {
            fprintf(stderr, "todo-c-client: timed out waiting for " LIST_TOPIC "\n");
            return -1;
        }
        bool ours = strcmp(topic, LIST_TOPIC) == 0;
        int r = ours ? decode_list(t, m->payload, (size_t)m->payloadlen, out) : 0;
        MQTTClient_freeMessage(&m);
        MQTTClient_free(topic);
        if (ours)
            return r;
    }
}

static int usage(void) {
    fprintf(stderr, "usage: todo-c-client [--host H] [--port N] [--xsd F] "
                    "list | add TITLE | done ID | remove ID\n");
    return 2;
}

int main(int argc, char **argv) {
    const char *verb = NULL, *arg = NULL;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--host") == 0 && i + 1 < argc)
            g_host = argv[++i];
        else if (strcmp(argv[i], "--port") == 0 && i + 1 < argc)
            g_port = atoi(argv[++i]);
        else if (strcmp(argv[i], "--xsd") == 0 && i + 1 < argc)
            g_xsd = argv[++i];
        else if (!verb)
            verb = argv[i];
        else if (!arg)
            arg = argv[i];
        else
            return usage();
    }
    if (!verb)
        return usage();
    bool is_list = strcmp(verb, "list") == 0;
    bool known = is_list || strcmp(verb, "add") == 0 || strcmp(verb, "done") == 0 ||
                 strcmp(verb, "remove") == 0;
    if (!known || (is_list != (arg == NULL)))
        return usage();

    RexiTables *t = NULL;
    const char *paths[] = {g_xsd};
    if (rexi_tables_from_xsd(paths, 1, &t) != REXI_STATUS_OK) {
        fprintf(stderr, "todo-c-client: %s: %s\n", g_xsd, rexi_last_error());
        return 1;
    }

    char uri[300], client_id[64];
    snprintf(uri, sizeof uri, "tcp://%s:%d", g_host, g_port);
    snprintf(client_id, sizeof client_id, "todo-c-%ld", (long)getpid());
    MQTTClient c;
    int status = 1;
    if (MQTTClient_create(&c, uri, client_id, MQTTCLIENT_PERSISTENCE_NONE, NULL) !=
        MQTTCLIENT_SUCCESS) {
        fprintf(stderr, "todo-c-client: cannot create the MQTT client\n");
        rexi_tables_free(t);
        return 1;
    }
    MQTTClient_connectOptions co = MQTTClient_connectOptions_initializer;
    co.MQTTVersion = MQTTVERSION_3_1_1;
    co.cleansession = 1;
    int rc = MQTTClient_connect(c, &co);
    if (rc != MQTTCLIENT_SUCCESS) {
        fprintf(stderr, "todo-c-client: cannot connect to %s (%d)\n", uri, rc);
        goto out;
    }
    /* The broker answers the subscription with the retained list. */
    if (MQTTClient_subscribe(c, LIST_TOPIC, 1) != MQTTCLIENT_SUCCESS) {
        fprintf(stderr, "todo-c-client: subscribe failed\n");
        goto out_connected;
    }
    TodoList list;
    if (next_list(c, t, &list) != 0)
        goto out_connected;
    if (!is_list) {
        list_free(&list);
        RexiBuffer cmd;
        if (encode_command(t, verb, arg, &cmd) != 0)
            goto out_connected;
        MQTTClient_deliveryToken tok;
        rc = MQTTClient_publish(c, CMD_TOPIC, (int)cmd.len, cmd.data, 1, 0, &tok);
        if (rc == MQTTCLIENT_SUCCESS)
            rc = MQTTClient_waitForCompletion(c, tok, TIMEOUT_MS);
        rexi_buffer_free(cmd);
        if (rc != MQTTCLIENT_SUCCESS) {
            fprintf(stderr, "todo-c-client: publish failed (%d)\n", rc);
            goto out_connected;
        }
        if (next_list(c, t, &list) != 0)
            goto out_connected;
    }
    print_list(&list);
    list_free(&list);
    status = 0;
out_connected:
    MQTTClient_disconnect(c, 1000);
out:
    MQTTClient_destroy(&c);
    rexi_tables_free(t);
    return status;
}
