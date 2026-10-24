#include "platform.h"

/* One schema contains both signatures. The target C compiler selects one. */
void check_platform_signatures(void) {
#ifdef _WIN32
    void *(*raw)(void *) = windows_handles_handle_api_raw;
    void *(*take)(void *) = windows_handles_handle_api_take;
    WindowsHandlesHandleConsumerVtable callback = {0};
    callback.windows_only = NULL;
#else
    int (*raw)(int) = windows_handles_handle_api_raw;
    int (*take)(int) = windows_handles_handle_api_take;
    WindowsHandlesHandleConsumerVtable callback = {0};
    callback.__reserved_1 = NULL;
#endif
    (void)raw;
    (void)take;
    (void)callback;
}
