#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include "ffier_test.h"

_Static_assert(sizeof(FtInputDeviceIds) == 8, "InputDeviceIds size");
_Static_assert(offsetof(FtInputDeviceIds, bus) == 0, "InputDeviceIds.bus");
_Static_assert(offsetof(FtInputDeviceIds, vendor) == 2, "InputDeviceIds.vendor");
_Static_assert(offsetof(FtInputDeviceIds, product) == 4, "InputDeviceIds.product");
_Static_assert(offsetof(FtInputDeviceIds, version) == 6, "InputDeviceIds.version");
_Static_assert(sizeof(FtInputAbsInfo) == 24, "InputAbsInfo size");
_Static_assert(offsetof(FtInputAbsInfo, value) == 0, "InputAbsInfo.value");
_Static_assert(offsetof(FtInputAbsInfo, minimum) == 4, "InputAbsInfo.minimum");
_Static_assert(offsetof(FtInputAbsInfo, maximum) == 8, "InputAbsInfo.maximum");
_Static_assert(offsetof(FtInputAbsInfo, fuzz) == 12, "InputAbsInfo.fuzz");
_Static_assert(offsetof(FtInputAbsInfo, flat) == 16, "InputAbsInfo.flat");
_Static_assert(offsetof(FtInputAbsInfo, resolution) == 20, "InputAbsInfo.resolution");
_Static_assert(sizeof(FtInputEvent) == 24, "InputEvent size");
_Static_assert(offsetof(FtInputEvent, device) == 0, "InputEvent.device");
_Static_assert(offsetof(FtInputEvent, kind) == 8, "InputEvent.kind");
_Static_assert(offsetof(FtInputEvent, code) == 12, "InputEvent.code");
_Static_assert(offsetof(FtInputEvent, flags) == 16, "InputEvent.flags");
_Static_assert(offsetof(FtInputEvent, value) == 20, "InputEvent.value");

static int test_count = 0;
#define RUN_TEST(fn) do { \
    printf("  %s ... ", #fn); \
    fn(); \
    printf("ok\n"); \
    test_count++; \
} while (0)

/* Helper: compare FtStr to C string literal */
static void assert_ft_str_eq(FtStr s, const char* expected) {
    assert(s.len == strlen(expected));
    assert(memcmp(s.data, expected, s.len) == 0);
}

/* ===================================================================== */
/* Constructor patterns                                                  */
/* ===================================================================== */

void static_method_returning_self(void) {
    FtWidget w = ft_widget_new();
    assert(w != NULL);
    assert(ft_widget_get_count(w) == 0);
    ft_widget_destroy(w);
}

void static_method_returning_self_with_str_param(void) {
    FtWidget w = ft_widget_with_name(FT_STR("hello"));
    assert(w != NULL);
    assert_ft_str_eq(ft_widget_name(w), "hello");
    ft_widget_destroy(w);
}

/* ===================================================================== */
/* Receiver patterns                                                     */
/* ===================================================================== */

void immutable_ref_method_returning_primitive(void) {
    FtWidget w = ft_widget_new();
    assert(ft_widget_get_count(w) == 0);
    ft_widget_destroy(w);
}

void mutable_ref_method_void_return(void) {
    FtWidget w = ft_widget_new();
    ft_widget_set_count(w, 42);
    assert(ft_widget_get_count(w) == 42);
    ft_widget_destroy(w);
}

void by_value_method_void_return(void) {
    FtWidget w = ft_widget_new();
    ft_widget_consume(w);
    /* w is consumed — no destroy needed */
}

/* ===================================================================== */
/* Primitive param/return types                                          */
/* ===================================================================== */

void method_returning_bool(void) {
    FtWidget w = ft_widget_new();
    assert(ft_widget_is_active(w) == true);
    ft_widget_destroy(w);
}

void method_with_i64_param_returning_i64(void) {
    FtWidget w = ft_widget_new();
    assert(ft_widget_negate(w, 42) == -42);
    assert(ft_widget_negate(w, -100) == 100);
    assert(ft_widget_negate(w, 0) == 0);
    ft_widget_destroy(w);
}

/* ===================================================================== */
/* String/bytes returns                                                  */
/* ===================================================================== */

void method_returning_str(void) {
    FtWidget w = ft_widget_new();
    assert_ft_str_eq(ft_widget_name(w), "widget");
    ft_widget_destroy(w);
}

void method_returning_bytes(void) {
    FtWidget w = ft_widget_with_name(FT_STR("abc"));
    FtBytes b = ft_widget_data(w);
    assert(b.len == 3);
    assert(b.data[0] == 'a');
    assert(b.data[1] == 'b');
    assert(b.data[2] == 'c');
    ft_widget_destroy(w);
}

void method_with_str_param_returning_str(void) {
    FtWidget w = ft_widget_new();
    assert_ft_str_eq(ft_widget_echo(w, FT_STR("ping")), "ping");
    ft_widget_destroy(w);
}

/* ===================================================================== */
/* Str slice param                                                       */
/* ===================================================================== */

void method_with_str_slice_param(void) {
    FtWidget w = ft_widget_new();
    FtStr tags[] = { FT_STR("alpha"), FT_STR("beta"), FT_STR("gamma") };
    ft_widget_set_tags(w, tags, 3);
    assert_ft_str_eq(ft_widget_tags_joined(w), "alpha,beta,gamma");
    ft_widget_destroy(w);
}

void method_with_bytes_param(void) {
    FtWidget w = ft_widget_new();
    uint8_t data[] = { 10, 20, 30, 40 };
    assert(ft_widget_sum_bytes(w, FT_BYTES(data)) == 100);
    ft_widget_destroy(w);
}

/* ===================================================================== */
/* File descriptor params                                                */
/* ===================================================================== */

void method_with_borrowed_fd_param(void) {
    FtWidget w = ft_widget_new();
    /* stdin is fd 0 */
    int fd_num = ft_widget_fd_number(w, STDIN_FILENO);
    assert(fd_num == STDIN_FILENO);
    ft_widget_destroy(w);
}

void method_with_borrowed_fd_returning_owned_fd(void) {
    FtWidget w = ft_widget_new();
    /* dup stdout */
    int new_fd = ft_widget_dup_fd(w, STDOUT_FILENO);
    assert(new_fd >= 0);
    assert(new_fd != STDOUT_FILENO);
    close(new_fd);
    ft_widget_destroy(w);
}

/* ===================================================================== */
/* Result return patterns                                                */
/* ===================================================================== */

void method_returning_result_void_ok(void) {
    FtWidget w = ft_widget_new();
    FtResult r = ft_widget_validate(w, NULL);
    assert(r == FT_RESULT_SUCCESS);
    ft_widget_destroy(w);
}

void method_returning_result_void_err(void) {
    FtWidget w = ft_widget_new();
    FtResult r = ft_widget_fail_always(w, NULL);
    assert(r == FT_ERROR_TEST_CUSTOM_MESSAGE);
    ft_widget_destroy(w);
}

void method_returning_result_value_ok(void) {
    FtWidget w = ft_widget_new();
    int32_t result = -1;
    FtResult r = ft_widget_parse_count(w, FT_STR("hello"), &result, NULL);
    assert(r == FT_RESULT_SUCCESS);
    assert(result == 5); /* len("hello") == 5 */
    ft_widget_destroy(w);
}

void method_returning_result_value_err(void) {
    FtWidget w = ft_widget_new();
    int32_t result = -1;
    FtResult r = ft_widget_parse_count(w, FT_STR("error"), &result, NULL);
    assert(r == FT_ERROR_TEST_NOT_FOUND);
    ft_widget_destroy(w);
}

void method_returning_result_str_ok(void) {
    FtWidget w = ft_widget_new();
    FtStr result = { 0 };
    FtResult r = ft_widget_describe(w, 0, &result, NULL);
    assert(r == FT_RESULT_SUCCESS);
    assert_ft_str_eq(result, "zero");
    ft_widget_destroy(w);
}

void method_returning_result_str_err(void) {
    FtWidget w = ft_widget_new();
    FtStr result = { 0 };
    FtResult r = ft_widget_describe(w, 99, &result, NULL);
    assert(r == FT_ERROR_TEST_NOT_FOUND);
    ft_widget_destroy(w);
}

void method_returning_result_handle_ok(void) {
    FtWidget w = ft_widget_new();
    ft_widget_set_count(w, 7);
    /* GLib-style: handle returned directly, NULL on error */
    FtGadget g = ft_widget_try_create_gadget(w, true, NULL);
    assert(g != NULL);
    assert(ft_gadget_get(g) == 7);
    ft_gadget_destroy(g);
    ft_widget_destroy(w);
}

void method_returning_result_handle_err(void) {
    FtWidget w = ft_widget_new();
    FtError err = NULL;
    FtGadget g = ft_widget_try_create_gadget(w, false, &err);
    assert(g == NULL);
    assert(err != NULL);
    FtResult r = ft_error_result(err);
    assert(r == FT_ERROR_TEST_NOT_FOUND);
    ft_error_destroy(err);
    ft_widget_destroy(w);
}

void method_returning_result_fail_with_value(void) {
    FtWidget w = ft_widget_new();
    int32_t result = -1;
    FtResult r = ft_widget_fail_with_value(w, &result, NULL);
    assert(r == FT_ERROR_TEST_INVALID_INPUT);
    ft_widget_destroy(w);
}

/* ===================================================================== */
/* Handle as parameter                                                   */
/* ===================================================================== */

void method_with_borrowed_handle_param(void) {
    FtWidget w = ft_widget_new();
    ft_widget_set_count(w, 10);
    FtGadget g = ft_widget_create_gadget(w);
    assert(ft_widget_read_gadget(w, g) == 10);
    ft_gadget_destroy(g);
    ft_widget_destroy(w);
}

void method_with_mutable_handle_param(void) {
    FtWidget w = ft_widget_new();
    FtGadget g = ft_widget_create_gadget(w);
    assert(ft_gadget_get(g) == 0);
    ft_widget_update_gadget(w, g, 99);
    assert(ft_gadget_get(g) == 99);
    ft_gadget_destroy(g);
    ft_widget_destroy(w);
}

void method_returning_handle(void) {
    FtWidget w = ft_widget_new();
    ft_widget_set_count(w, 33);
    FtGadget g = ft_widget_create_gadget(w);
    assert(g != NULL);
    assert(ft_gadget_get(g) == 33);
    ft_gadget_destroy(g);
    ft_widget_destroy(w);
}

/* ===================================================================== */
/* Builder pattern (by-value self -> Self)                               */
/* ===================================================================== */

void builder_method_returning_self(void) {
    FtConfig c = ft_config_new();
    ft_config_set_name(&c, FT_STR("myconfig"));
    ft_config_set_size(&c, 42);
    assert_ft_str_eq(ft_config_get_name(c), "myconfig");
    assert(ft_config_get_size(c) == 42);
    ft_config_destroy(c);
}

void builder_method_returning_result_self_ok(void) {
    FtConfig c = ft_config_new();
    ft_config_set_name(&c, FT_STR("valid"));
    FtResult r = ft_config_validated(&c, NULL);
    assert(r == FT_RESULT_SUCCESS);
    assert_ft_str_eq(ft_config_get_name(c), "valid");
    ft_config_destroy(c);
}

void builder_method_returning_result_self_err(void) {
    FtConfig c = ft_config_new();
    /* name is empty — validated() should fail */
    FtResult r = ft_config_validated(&c, NULL);
    assert(r == FT_ERROR_TEST_INVALID_INPUT);
    /* After error, the handle is consumed (by-value self).
     * The builder took ownership — don't destroy. */
}

void builder_consuming_self_returning_other_handle(void) {
    FtGizmoBuilder b = ft_gizmo_builder_new();
    ft_gizmo_builder_set_name(b, FT_STR("mygizmo"));
    ft_gizmo_builder_set_size(b, 100);
    /* build() consumes builder, returns Gizmo */
    FtGizmo g = ft_gizmo_builder_build(b);
    /* b is consumed — don't destroy */
    assert(g != NULL);
    assert_ft_str_eq(ft_gizmo_name(g), "mygizmo");
    assert(ft_gizmo_size(g) == 100);
    ft_gizmo_destroy(g);
}

void builder_consuming_self_returning_result_handle_ok(void) {
    FtGizmoBuilder b = ft_gizmo_builder_new();
    ft_gizmo_builder_set_name(b, FT_STR("valid"));
    ft_gizmo_builder_set_size(b, 50);
    /* GLib-style: handle returned directly */
    FtGizmo g = ft_gizmo_builder_try_build(b, NULL);
    /* b is consumed */
    assert(g != NULL);
    assert_ft_str_eq(ft_gizmo_name(g), "valid");
    assert(ft_gizmo_size(g) == 50);
    ft_gizmo_destroy(g);
}

void builder_consuming_self_returning_result_handle_err(void) {
    FtGizmoBuilder b = ft_gizmo_builder_new();
    /* name empty — try_build() should fail */
    FtError err = NULL;
    FtGizmo g = ft_gizmo_builder_try_build(b, &err);
    /* b is consumed */
    assert(g == NULL);
    assert(err != NULL);
    FtResult r = ft_error_result(err);
    assert(r == FT_ERROR_TEST_INVALID_INPUT);
    ft_error_destroy(err);
}

/* ===================================================================== */
/* Error type FFI                                                        */
/* ===================================================================== */

void error_result_constants(void) {
    assert(FT_RESULT_SUCCESS == 0);
    /* Constants have baked-in type tags — nonzero */
    assert(FT_ERROR_TEST_NOT_FOUND != 0);
    assert(FT_ERROR_TEST_CUSTOM_MESSAGE != 0);
    assert(FT_ERROR_TEST_INVALID_INPUT != 0);
    /* Different variants have different values */
    assert(FT_ERROR_TEST_NOT_FOUND != FT_ERROR_TEST_CUSTOM_MESSAGE);
}

void error_name_custom_message(void) {
    FtWidget w = ft_widget_new();
    FtResult r = ft_widget_fail_always(w, NULL);
    assert(r == FT_ERROR_TEST_CUSTOM_MESSAGE);
    const char* msg = ft_result_name_cstr(r);
    assert(msg != NULL);
    assert(strcmp(msg, "CustomMessage") == 0);
    ft_widget_destroy(w);
}

void error_name_not_found(void) {
    FtWidget w = ft_widget_new();
    int32_t result;
    FtResult r = ft_widget_parse_count(w, FT_STR("error"), &result, NULL);
    assert(r == FT_ERROR_TEST_NOT_FOUND);
    const char* msg = ft_result_name_cstr(r);
    assert(msg != NULL);
    assert(strcmp(msg, "NotFound(...)") == 0);
    ft_widget_destroy(w);
}

void error_name_success(void) {
    const char* msg = ft_result_name_cstr(FT_RESULT_SUCCESS);
    assert(msg != NULL);
    assert(strcmp(msg, "success") == 0);
}

/* Helper: PushStr callback that appends to a buffer */
struct push_str_buf {
    char data[256];
    size_t len;
};

static bool push_str_to_buf(void* self_data, FtStr s) {
    struct push_str_buf* buf = (struct push_str_buf*)self_data;
    if (buf->len + s.len < sizeof(buf->data)) {
        memcpy(buf->data + buf->len, s.data, s.len);
        buf->len += s.len;
    }
    return true;
}

/* Helper: construct a stack-local PushStr handle wrapping push_str_to_buf */
static void error_message_to_buf(FtError err, struct push_str_buf* buf) {
    buf->len = 0;
    FtPushStrVtable vtable = {
        .drop = NULL,
        .push = push_str_to_buf,
    };
    /* Stack-local FfierHandle layout: [type_tag(u32) | metadata(u32) | VtableHandle] */
    struct {
        uint32_t type_tag;
        uint32_t metadata;
        const void* vtable_ptr;
        const void* user_data;
        uint16_t vtable_size;
    } handle = {
        .type_tag = FT_PUSH_STR_TYPE_TAG,
        .metadata = 0,
        .vtable_ptr = &vtable,
        .user_data = buf,
        .vtable_size = sizeof(vtable),
    };
    if (err == NULL) return; /* self-dispatch doesn't accept null handles */
    ft_error_message(err, &handle);
}

void error_handle_message_and_destroy(void) {
    FtWidget w = ft_widget_new();
    FtError err = NULL;
    FtResult r = ft_widget_fail_always(w, &err);
    assert(r != FT_RESULT_SUCCESS);
    assert(err != NULL);
    struct push_str_buf buf;
    error_message_to_buf(err, &buf);
    assert(buf.len > 0);
    assert(buf.len == strlen("custom error message"));
    assert(memcmp(buf.data, "custom error message", buf.len) == 0);
    ft_error_destroy(err);
    ft_widget_destroy(w);
}

void error_handle_null_safe(void) {
    ft_error_destroy(NULL); /* should not crash */
}

/* ===================================================================== */
/* Vtable / implementable                                                */
/* ===================================================================== */

static int32_t test_process(void* self_data, int32_t input) {
    (void)self_data;
    return input * 2;
}

static FtStr test_processor_name(void* self_data) {
    (void)self_data;
    FtStr s = { .data = "test_proc", .len = 9 };
    return s;
}

static int g_drop_called = 0;

static void test_drop(void* self_data) {
    (void)self_data;
    g_drop_called = 1;
}

static const FtProcessorVtable g_test_vtable = {
    .drop = test_drop,
    .process = test_process,
    .name = test_processor_name,
};

/* Construct a heap-allocated processor handle (same layout as FfierHandle<VtableHandle>).
 * In real usage this would be a library-provided macro or function. */
static void* make_processor_handle(void* user_data) {
    struct {
        uint32_t type_tag;
        uint32_t metadata;
        const void* vtable_ptr;
        const void* user_data;
        uint16_t vtable_size;
    } *handle = malloc(sizeof(*handle));
    handle->type_tag = FT_PROCESSOR_TYPE_TAG;
    handle->metadata = 0;
    handle->vtable_ptr = &g_test_vtable;
    handle->user_data = user_data;
    handle->vtable_size = sizeof(g_test_vtable);
    return handle;
}

void vtable_constructor(void) {
    void* proc = make_processor_handle(NULL);
    assert(proc != NULL);
    /* Just test construction and destroy */
    ft_processor_destroy(proc);
}

void vtable_dyn_dispatch_process(void) {
    FtPipeline p = ft_pipeline_new();
    void* proc = make_processor_handle(NULL);
    ft_pipeline_run(p, proc, 21);
    assert(ft_pipeline_result_count(p) == 1);
    int32_t last = -1;
    FtResult r = ft_pipeline_last_result(p, &last, NULL);
    assert(r == FT_RESULT_SUCCESS);
    assert(last == 42);
    ft_pipeline_destroy(p);
}

void vtable_drop_callback(void) {
    g_drop_called = 0;
    void* proc = make_processor_handle(NULL);
    /* run() consumes the processor handle, which should trigger drop */
    FtPipeline p = ft_pipeline_new();
    ft_pipeline_run(p, proc, 1);
    assert(g_drop_called == 1);
    ft_pipeline_destroy(p);
}

/* ===================================================================== */
/* Fixed-layout values and optional return out-pointers                 */
/* ===================================================================== */

void value_struct_bridge_roundtrip(void) {
    FtEventQueue queue = ft_event_queue_new();
    FtInputEvent event = {
        .device = { .bus = 3, .vendor = 0x1af4, .product = 0x12, .version = 1 },
        .kind = FT_INPUT_EVENT_KIND_KEY,
        .code = 30,
        .flags = FT_INPUT_EVENT_FLAGS_REPEAT,
        .value = 19,
    };
    FtInputEvent echoed = ft_event_queue_echo_event(queue, event);
    assert(echoed.device.vendor == 0x1af4);
    assert(echoed.kind == FT_INPUT_EVENT_KIND_KEY);
    assert(echoed.flags == FT_INPUT_EVENT_FLAGS_REPEAT);
    assert(echoed.value == 19);

    const FtInputDeviceIds* ids = ft_event_queue_ids(queue);
    assert(ids->vendor == 0x1af4);
    FtInputDeviceIds replacement = *ids;
    replacement.vendor = 7;
    ft_event_queue_set_ids(queue, &replacement);
    assert(ft_event_queue_ids(queue)->vendor == 7);
    ft_event_queue_ids_mut(queue)->vendor = 9;
    ft_event_queue_copy_ids_to(queue, &replacement);
    assert(replacement.vendor == 9);
    assert(ft_event_queue_maybe_ids(queue, false) == NULL);
    assert(ft_event_queue_maybe_ids(queue, true)->vendor == 9);
    assert(ft_event_queue_optional_ids_vendor(queue, NULL) == 0);
    assert(ft_event_queue_optional_ids_vendor(queue, &replacement) == 9);
    replacement.vendor = 0;
    ft_event_queue_optional_ids_mut(queue, &replacement);
    assert(replacement.vendor == 9);
    ft_event_queue_optional_ids_mut(queue, NULL);

    FtInputEvent out = { .value = -1 };
    assert(ft_event_queue_pop_event(queue, &out));
    assert(out.value == 42);
    assert(ft_event_queue_pop_event(queue, &out));
    assert(out.value == 1);
    assert(!ft_event_queue_pop_event(queue, &out));
    assert(out.value == 1); /* None leaves the output untouched. */
    ft_event_queue_destroy(queue);
}

void optional_value_result_bridge(void) {
    FtEventQueue queue = ft_event_queue_new();
    bool is_some = false;
    FtInputEvent out = { .value = -1 };
    FtError error = NULL;
    FtResult status = ft_event_queue_next_event(queue, &is_some, &out, &error);
    assert(status == FT_RESULT_SUCCESS && is_some && out.value == 42);
    status = ft_event_queue_next_event(queue, &is_some, &out, &error);
    assert(status == FT_RESULT_SUCCESS && is_some && out.value == 1);
    status = ft_event_queue_next_event(queue, &is_some, &out, &error);
    assert(status == FT_RESULT_SUCCESS && !is_some && out.value == 1);
    ft_event_queue_fail_next(queue);
    status = ft_event_queue_next_event(queue, &is_some, &out, &error);
    assert(status != FT_RESULT_SUCCESS && error != NULL);
    ft_error_destroy(error);
    ft_event_queue_destroy(queue);
}

typedef struct {
    int calls;
    FtInputDeviceIds ids;
    FtInputAbsInfo abs;
    FtInputEvent submitted;
} InputCallbackState;

static FtResult callback_next_event(void* data, bool* is_some, FtInputEvent* result, FtError* error) {
    (void)error;
    InputCallbackState* state = data;
    if (state->calls++ == 0) {
        *is_some = true;
        *result = (FtInputEvent) { .device = state->ids, .kind = FT_INPUT_EVENT_KIND_KEY,
            .code = 30, .flags = 0, .value = 42 };
    } else if (state->calls == 2) {
        *is_some = true;
        *result = (FtInputEvent) { .device = state->ids, .kind = FT_INPUT_EVENT_KIND_KEY,
            .code = 31, .flags = 0, .value = 1 };
    } else {
        *is_some = false;
    }
    return FT_RESULT_SUCCESS;
}

static bool callback_poll_event(void* data, FtInputEvent* result) {
    InputCallbackState* state = data;
    if (state->calls++ == 0) {
        *result = state->submitted;
        return true;
    }
    return false;
}

static FtInputDeviceIds callback_device_ids(void* data) {
    return ((InputCallbackState*)data)->ids;
}

static const FtInputAbsInfo* callback_abs_info(void* data) {
    return &((InputCallbackState*)data)->abs;
}

static FtInputAbsInfo* callback_abs_info_mut(void* data) {
    return &((InputCallbackState*)data)->abs;
}

static const FtInputAbsInfo* callback_optional_abs_info(void* data, bool available) {
    return available ? &((InputCallbackState*)data)->abs : NULL;
}

static void callback_submit_event(void* data, FtInputEvent event) {
    ((InputCallbackState*)data)->submitted = event;
}

static void* make_input_handle(uint32_t tag, const void* vtable, size_t vtable_size, void* data) {
    struct {
        uint32_t type_tag;
        uint32_t metadata;
        const void* vtable_ptr;
        const void* user_data;
        uint16_t vtable_size;
    } *handle = malloc(sizeof(*handle));
    assert(handle != NULL);
    handle->type_tag = tag;
    handle->metadata = 0;
    handle->vtable_ptr = vtable;
    handle->user_data = data;
    handle->vtable_size = (uint16_t)vtable_size;
    return handle;
}

void value_struct_vtable_callbacks(void) {
    InputCallbackState state = {
        .ids = { .bus = 3, .vendor = 0x1af4, .product = 0x12, .version = 1 },
        .abs = { .value = 42, .minimum = 0, .maximum = 255, .resolution = 1 },
    };
    const FtInputBackendVtable backend_vtable = { .next_event = callback_next_event };
    FtInputBackend backend = make_input_handle(
        FT_INPUT_BACKEND_TYPE_TAG, &backend_vtable, sizeof(backend_vtable), &state);
    int32_t total = 0;
    FtResult status = ft_drain_input_backend(backend, &total, NULL);
    assert(status == FT_RESULT_SUCCESS && total == 43);
    assert(state.calls == 3);

    state.calls = 0;
    const FtInputSourceVtable source_vtable = {
        .poll_event = callback_poll_event,
        .device_ids = callback_device_ids,
        .abs_info = callback_abs_info,
        .abs_info_mut = callback_abs_info_mut,
        .optional_abs_info = callback_optional_abs_info,
        .submit_event = callback_submit_event,
    };
    FtInputSource source = make_input_handle(
        FT_INPUT_SOURCE_TYPE_TAG, &source_vtable, sizeof(source_vtable), &state);
    assert(ft_probe_input_source(source) == 6993);
    assert(state.calls == 3);
    assert(state.abs.value == 43);
    assert(state.submitted.value == 7);
}

/* ===================================================================== */
/* Lifetime-parameterized types                                          */
/* ===================================================================== */

void lifetime_type_borrowing_handle(void) {
    FtWidget w = ft_widget_new();
    ft_widget_set_count(w, 77);
    FtView v = ft_view_create(w);
    assert(v != NULL);
    assert(ft_view_source_count(v) == 77);
    ft_view_destroy(v);
    ft_widget_destroy(w);
}

void lifetime_type_reading_through_borrow(void) {
    FtWidget w = ft_widget_new();
    ft_widget_set_count(w, 123);
    FtView v = ft_view_create(w);
    assert(ft_view_source_count(v) == 123);
    ft_view_destroy(v);
    ft_widget_destroy(w);
}

void lifetime_type_str_methods(void) {
    FtWidget w = ft_widget_new();
    FtView v = ft_view_create(w);
    assert_ft_str_eq(ft_view_label(v), "default");
    ft_view_set_label(v, FT_STR("custom"));
    assert_ft_str_eq(ft_view_label(v), "custom");
    ft_view_destroy(v);
    ft_widget_destroy(w);
}

/* ===================================================================== */
/* Optional fd (Result<Option<BorrowedFd>, E>)                          */
/* ===================================================================== */

void method_returning_result_optional_fd_some(void) {
    FtWidget w = ft_widget_new();
    int fd = -1;
    FtResult r = ft_widget_maybe_fd(w, 1, &fd, NULL);
    assert(r == FT_RESULT_SUCCESS);
    assert(fd == 0); /* stdin */
    ft_widget_destroy(w);
}

void method_returning_result_optional_fd_none(void) {
    FtWidget w = ft_widget_new();
    int fd = 99;
    FtResult r = ft_widget_maybe_fd(w, 0, &fd, NULL);
    assert(r == FT_RESULT_SUCCESS);
    assert(fd == -1); /* None sentinel */
    ft_widget_destroy(w);
}

void method_returning_result_optional_fd_err(void) {
    FtWidget w = ft_widget_new();
    int fd = 99;
    FtResult r = ft_widget_maybe_fd(w, -1, &fd, NULL);
    assert(r == FT_ERROR_TEST_INVALID_INPUT);
    ft_widget_destroy(w);
}

/* ===================================================================== */
/* Destroy                                                               */
/* ===================================================================== */

void destroy_handle(void) {
    FtWidget w = ft_widget_new();
    ft_widget_destroy(w);
    /* No crash means success */
}

void destroy_null_handle(void) {
    /* destroy(NULL) should be safe */
    ft_widget_destroy(NULL);
}

/* ===================================================================== */
/* Main                                                                  */
/* ===================================================================== */

int main(void) {
    printf("Running ffier C integration tests...\n");

    printf("\n[constructors]\n");
    RUN_TEST(static_method_returning_self);
    RUN_TEST(static_method_returning_self_with_str_param);

    printf("\n[receivers]\n");
    RUN_TEST(immutable_ref_method_returning_primitive);
    RUN_TEST(mutable_ref_method_void_return);
    RUN_TEST(by_value_method_void_return);

    printf("\n[primitive types]\n");
    RUN_TEST(method_returning_bool);
    RUN_TEST(method_with_i64_param_returning_i64);

    printf("\n[string/bytes]\n");
    RUN_TEST(method_returning_str);
    RUN_TEST(method_returning_bytes);
    RUN_TEST(method_with_str_param_returning_str);

    printf("\n[str slice param]\n");
    RUN_TEST(method_with_str_slice_param);
    RUN_TEST(method_with_bytes_param);

    printf("\n[file descriptors]\n");
    RUN_TEST(method_with_borrowed_fd_param);
    RUN_TEST(method_with_borrowed_fd_returning_owned_fd);

    printf("\n[result returns]\n");
    RUN_TEST(method_returning_result_void_ok);
    RUN_TEST(method_returning_result_void_err);
    RUN_TEST(method_returning_result_value_ok);
    RUN_TEST(method_returning_result_value_err);
    RUN_TEST(method_returning_result_str_ok);
    RUN_TEST(method_returning_result_str_err);
    RUN_TEST(method_returning_result_handle_ok);
    RUN_TEST(method_returning_result_handle_err);
    RUN_TEST(method_returning_result_fail_with_value);

    printf("\n[handle params]\n");
    RUN_TEST(method_with_borrowed_handle_param);
    RUN_TEST(method_with_mutable_handle_param);
    RUN_TEST(method_returning_handle);

    printf("\n[builder pattern]\n");
    RUN_TEST(builder_method_returning_self);
    RUN_TEST(builder_method_returning_result_self_ok);
    RUN_TEST(builder_method_returning_result_self_err);
    RUN_TEST(builder_consuming_self_returning_other_handle);
    RUN_TEST(builder_consuming_self_returning_result_handle_ok);
    RUN_TEST(builder_consuming_self_returning_result_handle_err);

    printf("\n[error type]\n");
    RUN_TEST(error_result_constants);
    RUN_TEST(error_name_custom_message);
    RUN_TEST(error_name_not_found);
    RUN_TEST(error_name_success);
    RUN_TEST(error_handle_message_and_destroy);
    RUN_TEST(error_handle_null_safe);

    printf("\n[vtable/implementable]\n");
    RUN_TEST(vtable_dyn_dispatch_process);

    RUN_TEST(vtable_drop_callback);

    printf("\n[value structs]\n");
    RUN_TEST(value_struct_bridge_roundtrip);
    RUN_TEST(optional_value_result_bridge);
    RUN_TEST(value_struct_vtable_callbacks);

    printf("\n[lifetime types]\n");
    RUN_TEST(lifetime_type_borrowing_handle);
    RUN_TEST(lifetime_type_reading_through_borrow);
    RUN_TEST(lifetime_type_str_methods);

    printf("\n[optional fd]\n");
    RUN_TEST(method_returning_result_optional_fd_some);
    RUN_TEST(method_returning_result_optional_fd_none);
    RUN_TEST(method_returning_result_optional_fd_err);

    printf("\n[destroy]\n");
    RUN_TEST(destroy_handle);
    RUN_TEST(destroy_null_handle);

    printf("\nAll %d tests passed!\n", test_count);
    return 0;
}
