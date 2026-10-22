#ifndef ZUN_NAPI_H
#define ZUN_NAPI_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ZunHost ZunHost;

enum {
    ZUN_HOST_OK = 0,
    ZUN_HOST_INVALID_ARGUMENT = 1,
    ZUN_HOST_VM_ERROR = 2,
    ZUN_HOST_BUFFER_TOO_SMALL = 3
};

ZunHost *zun_host_create(void);
void zun_host_destroy(ZunHost *host);

int32_t zun_host_execute_i64(
    ZunHost *host,
    const uint8_t *module_ptr,
    size_t module_len,
    int64_t *out);

int32_t zun_host_execute_u64(
    ZunHost *host,
    const uint8_t *module_ptr,
    size_t module_len,
    uint64_t *out);

int32_t zun_host_execute_f64(
    ZunHost *host,
    const uint8_t *module_ptr,
    size_t module_len,
    double *out);

int32_t zun_host_execute_bool(
    ZunHost *host,
    const uint8_t *module_ptr,
    size_t module_len,
    bool *out);

int32_t zun_host_execute_string(
    ZunHost *host,
    const uint8_t *module_ptr,
    size_t module_len,
    uint8_t *out,
    size_t out_len,
    size_t *written);

#ifdef __cplusplus
}
#endif

#endif
