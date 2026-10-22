#ifndef ZUN_ABI_H
#define ZUN_ABI_H

#include <stddef.h>
#include <stdint.h>

#define ZUN_ABI_V1 UINT32_C(1)

typedef enum ZunTy {
    ZUN_TY_VOID = 0,
    ZUN_TY_BOOL = 1,
    ZUN_TY_I32 = 2,
    ZUN_TY_U32 = 3,
    ZUN_TY_I64 = 4,
    ZUN_TY_U64 = 5,
    ZUN_TY_F64 = 6,
    ZUN_TY_STR = 7,
    ZUN_TY_OPAQUE = 8
} ZunTy;

typedef struct ZunValue {
    uint32_t ty;
    uint32_t _reserved;
    uint64_t payload;
} ZunValue;

typedef int32_t (*ZunInvoke)(const ZunValue *args, size_t arg_count, ZunValue *out);

typedef struct ZunFunction {
    const char *key;
    const uint32_t *params;
    size_t param_count;
    uint32_t ret;
    ZunInvoke invoke;
} ZunFunction;

typedef int32_t (*ZunRegisterFunction)(void *context, const char *module,
                                      const ZunFunction *function);

typedef struct ZunRegistry {
    uint32_t abi_version;
    void *context;
    ZunRegisterFunction register_function;
} ZunRegistry;

typedef int32_t (*ZunRegisterV1)(ZunRegistry *registry);

#endif
