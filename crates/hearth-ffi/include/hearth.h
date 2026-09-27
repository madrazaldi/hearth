// C interface to Hearth's Rust core (crates/hearth-ffi). Strings are UTF-8
// JSON. Commands and their arguments match the Tauri commands in
// src-tauri/src/lib.rs (camelCase argument names; `upload_bytes` takes
// `data` as base64). Callbacks run on background threads.

#ifndef HEARTH_H
#define HEARTH_H

#include <stdbool.h>

typedef struct HearthHandle HearthHandle;

// `json` is an array of events, as the Svelte frontend receives them.
typedef void (*hearth_event_cb)(void *ctx, const char *json);
// `json` is the result, or a JSON string holding the error when !ok.
typedef void (*hearth_reply_cb)(void *ctx, bool ok, const char *json);

// Returns NULL on failure and sets *error (free it with hearth_string_free).
HearthHandle *hearth_open(const char *config_dir, const char *data_dir, hearth_event_cb on_event, void *ctx,
                          char **error);
// `reply` is called exactly once.
void hearth_call(const HearthHandle *handle, const char *command, const char *args_json, hearth_reply_cb reply,
                 void *ctx);
bool hearth_close_to_tray(const HearthHandle *handle);
// Quits every connection and flushes the log. The handle is invalid afterwards.
void hearth_close(HearthHandle *handle);
void hearth_string_free(char *s);

#endif
