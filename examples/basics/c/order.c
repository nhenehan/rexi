/*
 * Rust only (C API): writes the order document event by event, then reads it
 * back with the pull decoder. This is write_events.rs and read_events.rs in C,
 * and it prints the same lines as read_events.
 *
 * Every call returns a RexiStatus. Anything other than REXI_STATUS_OK (or
 * REXI_STATUS_END from the decoder) is an error, and rexi_last_error() says
 * what went wrong. Build it with the CMakeLists.txt next to this file.
 */
#include <stdio.h>
#include <stdlib.h>

#include "rexi.h"

static const char *NS = "urn:rexi:interop:order";

/* ANCHOR: write */
/* Stops the program with rexi's error message if a call failed. */
static void check(RexiStatus s) {
  if (s != REXI_STATUS_OK) {
    fprintf(stderr, "rexi: %s\n", rexi_last_error());
    exit(1);
  }
}

/* Writes <name>text</name>; the text is parsed with the schema's datatype. */
static void text_element(RexiEncoder *enc, const char *name, const char *text) {
  check(rexi_start_element(enc, NS, name));
  check(rexi_characters_lexical(enc, text));
  check(rexi_end_element(enc));
}

/* ANCHOR_END: write */

/* ---- reading: one callback per kind of event ---------------------------- */

typedef struct {
  int depth;
} Reader;

static void indent(const Reader *r) {
  for (int i = 0; i < r->depth; i++)
    fputs("  ", stdout);
}

static const char *kind_name(const RexiValue *v) {
  switch (v->kind) {
  case REXI_VALUE_STRING:
    return "string";
  case REXI_VALUE_BOOLEAN:
    return "boolean";
  case REXI_VALUE_INTEGER:
    return "integer";
  case REXI_VALUE_DECIMAL:
    return "decimal";
  default:
    return "other";
  }
}

static int on_sd(void *u) {
  (void)u;
  puts("start document");
  return 0;
}

static int on_ed(void *u) {
  (void)u;
  puts("end document");
  return 0;
}

/* ANCHOR: read */
static int on_se(RexiStr uri, RexiStr local, void *u) {
  Reader *r = u;
  (void)uri;
  indent(r);
  printf("%.*s\n", (int)local.len, local.ptr);
  r->depth++;
  return 0;
}

static int on_ee(void *u) {
  ((Reader *)u)->depth--;
  return 0;
}

/* Every decoded value also comes with its text form in `lexical`. */
static int on_at(RexiStr uri, RexiStr local, const RexiValue *v, void *u) {
  (void)uri;
  indent(u);
  printf("@%.*s = %.*s (%s)\n", (int)local.len, local.ptr, (int)v->lexical_len,
         v->lexical, kind_name(v));
  return 0;
}

static int on_ch(const RexiValue *v, void *u) {
  indent(u);
  printf("%.*s (%s)\n", (int)v->lexical_len, v->lexical, kind_name(v));
  return 0;
}
/* ANCHOR_END: read */

int main(int argc, char **argv) {
  const char *xsd = argc > 1 ? argv[1] : "data/order.xsd";
  RexiTables *tables = NULL;
  check(rexi_tables_from_xsd(&xsd, 1, &tables));

  /* ---- writing ---- */
  RexiOptions options = rexi_options_strict();
  RexiEncoder *enc = NULL;
  check(rexi_encoder_new(tables, &options, &enc));
  check(rexi_start_document(enc));
  check(rexi_start_element(enc, NS, "Order"));
  check(rexi_attribute_lexical(enc, "", "orderId", "ORD-2026-0007"));
  check(rexi_attribute_lexical(enc, "", "priority", "true"));

  check(rexi_start_element(enc, NS, "Customer"));
  check(rexi_attribute_lexical(enc, "", "loyaltyId", "L-8821"));
  text_element(enc, "Name", "Raven Bookstore");
  text_element(enc, "Tier", "gold");
  check(rexi_end_element(enc));

  static const char *items[][4] = {
      {"BK-001", "Distributed Systems Patterns", "2", "42.95"},
      {"BK-014", "Practical EXI", "1", "31.50"},
  };
  for (size_t i = 0; i < sizeof items / sizeof items[0]; i++) {
    check(rexi_start_element(enc, NS, "Item"));
    check(rexi_attribute_lexical(enc, "", "sku", items[i][0]));
    text_element(enc, "Description", items[i][1]);
    text_element(enc, "Quantity", items[i][2]);
    text_element(enc, "Price", items[i][3]);
    check(rexi_end_element(enc));
  }

  text_element(enc, "Notes", "gift-wrap requested");
  check(rexi_end_element(enc));
  check(rexi_end_document(enc));

  /* finish frees the encoder and hands over the bytes. */
  RexiBuffer exi = {0};
  check(rexi_encoder_finish(enc, &exi));
  printf("EXI: %zu bytes\n", exi.len);

  /* ---- reading ---- */
  static const RexiContentHandler handler = {
      .start_document = on_sd,
      .end_document = on_ed,
      .start_element = on_se,
      .end_element = on_ee,
      .attribute = on_at,
      .characters = on_ch,
  };
  Reader reader = {0};
  RexiDecoder *dec = NULL;
  check(rexi_decoder_new(tables, exi.data, exi.len, &options, &dec));
  RexiStatus s;
  while ((s = rexi_decoder_next(dec, &handler, &reader)) == REXI_STATUS_OK) {
  }
  if (s != REXI_STATUS_END)
    check(s);

  rexi_decoder_free(dec);
  rexi_buffer_free(exi);
  rexi_tables_free(tables);
  return 0;
}
