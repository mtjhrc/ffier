#include "platform.h"

/* One schema contains both signatures. The target C compiler selects one. */
void check_platform_signatures(void) {
#ifdef WINDOWS_HANDLES_CFG_FEATURE_WIDE
    int64_t (*width)(int64_t) = windows_handles_feature_width;
    int64_t (*method_width)(WindowsHandlesHandleApi, int64_t) = windows_handles_handle_api_feature_width;
#else
    int32_t (*width)(int32_t) = windows_handles_feature_width;
    int32_t (*method_width)(WindowsHandlesHandleApi, int32_t) = windows_handles_handle_api_feature_width;
#endif
    int32_t (*generated)(void) = windows_handles_generated_value;
    int32_t (*conditional)(void) = windows_handles_conditional_export;
#ifdef _WIN32
    void *(*raw)(void *) = windows_handles_handle_api_raw;
    void *(*take)(void *) = windows_handles_handle_api_take;
    void *(*resource)(void *) = windows_handles_resource;
    void *(*echo)(void *) = windows_handles_handle_api_echo;
    WindowsHandlesHandleConsumerVtable callback = {0};
    callback.windows_only = NULL;
#else
    int (*raw)(int) = windows_handles_handle_api_raw;
    int (*take)(int) = windows_handles_handle_api_take;
    int (*resource)(int) = windows_handles_resource;
    int (*echo)(int) = windows_handles_handle_api_echo;
    WindowsHandlesHandleConsumerVtable callback = {0};
    callback.__reserved_1 = NULL;
#endif
    (void)resource;
    (void)echo;
    (void)raw;
    (void)take;
    (void)callback;
    (void)width;
    (void)method_width;
    (void)generated;
    (void)conditional;
}
