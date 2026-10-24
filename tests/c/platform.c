#include "platform.h"

/* One schema contains both signatures. The target C compiler selects one. */
void check_platform_signatures(void) {
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
}
