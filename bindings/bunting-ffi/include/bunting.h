#ifndef BUNTING_H
#define BUNTING_H

#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

typedef struct BuntingFfiHandle BuntingFfiHandle;

typedef struct BuntingFfiError {
  int32_t code;
  char *message;
} BuntingFfiError;

struct BuntingFfiHandle *bunting_handle_new(void);

/**
 * # Safety
 *
 * `handle` must be null or a pointer returned by `bunting_handle_new` that
 * has not already been freed.
 */
void bunting_handle_free(struct BuntingFfiHandle *handle);

/**
 * # Safety
 *
 * `archive_json` must be a valid NUL-terminated string, `output_json` must be
 * writable, and `error` may be null or writable. Returned strings must be
 * released with `bunting_string_free`.
 */
int32_t bunting_replay_archive(const struct BuntingFfiHandle *handle,
                               const char *archive_json,
                               char **output_json,
                               struct BuntingFfiError *error);

/**
 * # Safety
 *
 * `value` must be null or a pointer returned by this library that has not
 * already been freed.
 */
void bunting_string_free(char *value);

#endif  /* BUNTING_H */
