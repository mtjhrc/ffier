#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "platform.h"

#ifndef _WIN32
#include <unistd.h>
#else
#include <windows.h>
#endif

/* One schema contains both signatures. The target C compiler selects one. */
void check_platform_signatures(void) {
#ifdef PLATFORM_MATRIX_CFG_FEATURE_WIDE
    int64_t (*width)(int64_t) = platform_matrix_feature_width;
    int64_t (*method_width)(PlatformMatrixHandleApi, int64_t) = platform_matrix_handle_api_feature_width;
#else
    int32_t (*width)(int32_t) = platform_matrix_feature_width;
    int32_t (*method_width)(PlatformMatrixHandleApi, int32_t) = platform_matrix_handle_api_feature_width;
#endif
    int32_t (*generated)(void) = platform_matrix_generated_value;
    int32_t (*conditional)(void) = platform_matrix_conditional_export;
#ifdef _WIN32
    void *(*raw)(void *) = platform_matrix_handle_api_raw;
    void *(*take)(void *) = platform_matrix_handle_api_take;
    void *(*resource)(void *) = platform_matrix_resource;
    void *(*echo)(void *) = platform_matrix_handle_api_echo;
    PlatformMatrixHandleConsumerVtable callback = {0};
    callback.windows_only = NULL;
#else
    int (*raw)(int) = platform_matrix_handle_api_raw;
    int (*take)(int) = platform_matrix_handle_api_take;
    int (*resource)(int) = platform_matrix_resource;
    int (*echo)(int) = platform_matrix_handle_api_echo;
    PlatformMatrixHandleConsumerVtable callback = {0};
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

/* Resource conversions. The general suites stay platform-agnostic, so the
 * descriptor and handle shapes are exercised here instead. */
void check_resource_conversions(void) {
    PlatformMatrixHandleApi api = platform_matrix_handle_api_new();

    /* Borrowed resource to raw value. */
#ifdef _WIN32
    HANDLE stdin_handle = GetStdHandle(STD_INPUT_HANDLE);
    assert(platform_matrix_handle_api_fd_number(api, stdin_handle) == (intptr_t)stdin_handle);
#else
    assert(platform_matrix_handle_api_fd_number(api, STDIN_FILENO) == STDIN_FILENO);
#endif

    /* Optional input, both directions. */
#ifdef _WIN32
    assert(platform_matrix_handle_api_fd_number_optional(api, NULL) == 0);
#else
    assert(platform_matrix_handle_api_fd_number_optional(api, -1) == -1);
#endif

    /* Owned duplicate of a borrowed resource. */
#ifdef _WIN32
    HANDLE stdout_handle = GetStdHandle(STD_OUTPUT_HANDLE);
    HANDLE new_handle = platform_matrix_handle_api_dup_fd(api, stdout_handle);
    assert(new_handle != NULL);
    assert(new_handle != stdout_handle);
    CloseHandle(new_handle);
#else
    int new_fd = platform_matrix_handle_api_dup_fd(api, STDOUT_FILENO);
    assert(new_fd >= 0);
    assert(new_fd != STDOUT_FILENO);
    close(new_fd);
#endif

    /* Result<Option<resource>, E>: success, none, and error. */
#ifdef _WIN32
    void *out = NULL;
    assert(platform_matrix_handle_api_maybe_fd(api, 1, &out, NULL) == PLATFORM_MATRIX_RESULT_SUCCESS);
    assert(out == stdin_handle);
    out = (void *)1;
    assert(platform_matrix_handle_api_maybe_fd(api, 0, &out, NULL) == PLATFORM_MATRIX_RESULT_SUCCESS);
    assert(out == NULL);
    out = (void *)1;
    assert(platform_matrix_handle_api_maybe_fd(api, -1, &out, NULL) == PLATFORM_MATRIX_ERROR_RESOURCE_INVALID_INPUT);
#else
    int out = -1;
    assert(platform_matrix_handle_api_maybe_fd(api, 1, &out, NULL) == PLATFORM_MATRIX_RESULT_SUCCESS);
    assert(out == 0); /* stdin */
    out = 99;
    assert(platform_matrix_handle_api_maybe_fd(api, 0, &out, NULL) == PLATFORM_MATRIX_RESULT_SUCCESS);
    assert(out == -1);
    out = 99;
    assert(platform_matrix_handle_api_maybe_fd(api, -1, &out, NULL) == PLATFORM_MATRIX_ERROR_RESOURCE_INVALID_INPUT);
#endif

    platform_matrix_handle_api_destroy(api);
}

int main(void) {
    check_platform_signatures();
    check_resource_conversions();
    printf("platform: resource conversions ok\n");
    return 0;
}
