#include <assert.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>
#include "ffier_test.h"

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
/* Fixed-layout value structs                                            */
/* ===================================================================== */

void value_struct_inline_and_by_pointer(void) {
    FtInputEvent event = ft_event_make(FT_LOG_LEVEL_WARN, 42);
    assert(event.sequence == 42);
    assert(event.level == FT_LOG_LEVEL_WARN);
    assert(event.payload.code == 0);
    assert(ft_event_sequence(event) == 42);
    assert(ft_event_payload_code(&event) == 0);
    ft_event_bump_payload_code(&event);
    assert(ft_event_payload_code(&event) == 1);
}

void value_struct_result_return(void) {
    FtEventPayload payload;
    FtError err = NULL;
    assert(ft_event_checked(7, &payload, &err) == FT_RESULT_SUCCESS);
    assert(err == NULL);
    assert(payload.code == 7);

    assert(ft_event_checked(0, &payload, &err) != FT_RESULT_SUCCESS);
    assert(err != NULL);
    ft_error_destroy(err);
}

/* Result<Option<ValueStruct>, E>: presence flag, value, then error out-param.
 * None must leave the value untouched; errors must leave both success
 * outputs untouched. */
void value_struct_optional_return(void) {
    FtFixedSource source = ft_fixed_source_new();

    FtInputEvent event = {0};
    bool is_some = false;
    FtError err = NULL;
    assert(ft_fixed_source_peek_sample(source, &is_some, &event, &err) == FT_RESULT_SUCCESS);
    assert(err == NULL);
    assert(is_some);
    assert(event.sequence == 7);
    assert(event.payload.code == 11);
    assert(event.level == FT_LOG_LEVEL_WARN);

    /* None: the flag clears and the value buffer is left alone. */
    FtInputEvent untouched = {0};
    untouched.sequence = 999;
    is_some = true;
    assert(ft_fixed_source_peek_sample(source, &is_some, &untouched, &err) == FT_RESULT_SUCCESS);
    assert(err == NULL);
    assert(!is_some);
    assert(untouched.sequence == 999);

    /* Error: neither success output is written. */
    is_some = true;
    untouched.sequence = 999;
    assert(ft_fixed_source_peek_sample(source, &is_some, &untouched, &err) != FT_RESULT_SUCCESS);
    assert(err != NULL);
    assert(is_some);
    assert(untouched.sequence == 999);
    ft_error_destroy(err);

    ft_fixed_source_destroy(source);
}

/* Free function with the same shape: proves the signature is identical. */
void value_struct_optional_return_free_function(void) {
    FtInputEvent sample = {0};
    bool is_some = false;
    FtError err = NULL;

    assert(ft_sample_at(0, &is_some, &sample, &err) == FT_RESULT_SUCCESS);
    assert(err == NULL);
    assert(is_some);
    assert(sample.sequence == 3);
    assert(sample.payload.code == 5);

    sample.sequence = 999;
    is_some = true;
    assert(ft_sample_at(1, &is_some, &sample, &err) == FT_RESULT_SUCCESS);
    assert(err == NULL);
    assert(!is_some);
    assert(sample.sequence == 999);

    is_some = true;
    sample.sequence = 999;
    assert(ft_sample_at(2, &is_some, &sample, &err) != FT_RESULT_SUCCESS);
    assert(err != NULL);
    assert(is_some);
    assert(sample.sequence == 999);
    ft_error_destroy(err);
}

void value_struct_inline_field_of_struct(void) {
    /* Nested value struct used as an inline field. */
    FtInputEvent event = {0};
    event.payload.code = 11;
    event.payload.weight = 2.5f;
    assert(ft_event_payload_code(&event) == 11);
    ft_event_bump_payload_code(&event);
    assert(event.payload.code == 12);
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
    FtStr message = ft_result_message(r);
    assert(message.len == strlen("custom error message"));
    assert(memcmp(message.data, "custom error message", message.len) == 0);
    ft_widget_destroy(w);
}

void error_name_not_found(void) {
    FtWidget w = ft_widget_new();
    int32_t result;
    FtResult r = ft_widget_parse_count(w, FT_STR("error"), &result, NULL);
    assert(r == FT_ERROR_TEST_NOT_FOUND);
    const char* msg = ft_result_name_cstr(r);
    assert(msg != NULL);
    assert(strcmp(msg, "NotFound") == 0);
    FtStr message = ft_result_message(r);
    assert(message.len == strlen("NotFound"));
    assert(memcmp(message.data, "NotFound", message.len) == 0);
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
    FtPushStrVtable vtable = FT_VTABLE(FtPushStrVtable,
        .drop = NULL,
        .push = push_str_to_buf,
    );
    FtVtableHandle handle = FT_TEMP_OBJ(FT_PUSH_STR_TYPE_TAG, &vtable, buf);
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

void error_factory_constructs_fieldless_variants_only(void) {
    FtResult results[] = { FT_ERROR_TEST_CUSTOM_MESSAGE, FT_ERROR_TEST_INVALID_INPUT };
    for (size_t i = 0; i < sizeof(results) / sizeof(results[0]); i++) {
        size_t before = ft_test_error_drop_count();
        FtError err = ft_error_new(results[i]);
        assert(err != NULL);
        assert(ft_error_result(err) == results[i]);
        struct push_str_buf buf;
        error_message_to_buf(err, &buf);
        assert(buf.len > 0);
        assert(ft_test_error_drop_count() == before);
        ft_error_destroy(err);
        assert(ft_test_error_drop_count() == before + 1);
    }
    assert(ft_error_new(FT_RESULT_SUCCESS) == NULL);
    assert(ft_error_new(FT_ERROR_TEST_NOT_FOUND) == NULL);
    assert(ft_error_new(FT_ERROR_TEST_NUMERIC_ERROR) == NULL);
    assert(ft_error_new(FT_ERROR_TEST_FATAL) == NULL);
    assert(ft_error_new(FT_ERROR_TEST_MIXED_ERROR) == NULL);
    assert(ft_error_new(FT_ERROR_TEST_OPAQUE_UNIT) == NULL);
    assert(ft_error_new(UINT64_MAX) == NULL);
}

void error_payload_uses_native_field_indices(void) {
    FtWidget widget = ft_widget_new();
    FtError error = NULL;
    assert(ft_widget_fail_with_mixed(widget, &error) == FT_ERROR_TEST_MIXED_ERROR);
    assert(error != NULL);
    int32_t first = 0;
    uint64_t last = 0;
    ft_error_payload(error, 0, &first, sizeof(first));
    ft_error_payload(error, 2, &last, sizeof(last));
    assert(first == 17);
    assert(last == 900);
    ft_error_destroy(error);
    ft_widget_destroy(widget);
}

static void assert_payload_access_aborts(FtError error, size_t index, size_t buf_size) {
    pid_t child = fork();
    assert(child >= 0);
    if (child == 0) {
        struct rlimit limit = { 0, 0 };
        assert(setrlimit(RLIMIT_CORE, &limit) == 0);
        assert(freopen("/dev/null", "w", stderr) != NULL);
        uint64_t output = 0;
        ft_error_payload(error, index, &output, buf_size);
        _exit(0);
    }
    int status;
    assert(waitpid(child, &status, 0) == child);
    assert(WIFSIGNALED(status));
    assert(WTERMSIG(status) == SIGABRT);
}

void error_payload_invalid_access_aborts(void) {
    FtWidget widget = ft_widget_new();
    FtError error = NULL;
    assert(ft_widget_fail_with_mixed(widget, &error) == FT_ERROR_TEST_MIXED_ERROR);
    assert(error != NULL);
    assert_payload_access_aborts(error, 1, sizeof(uint64_t));
    assert_payload_access_aborts(error, 3, sizeof(uint64_t));
    assert_payload_access_aborts(error, SIZE_MAX, sizeof(uint64_t));
    assert_payload_access_aborts(error, 0, 0);
    ft_error_destroy(error);
    ft_widget_destroy(widget);

    FtResult fieldless[] = { FT_ERROR_TEST_CUSTOM_MESSAGE, FT_ERROR_TEST_INVALID_INPUT };
    for (size_t i = 0; i < sizeof(fieldless) / sizeof(fieldless[0]); i++) {
        error = ft_error_new(fieldless[i]);
        assert(error != NULL);
        assert_payload_access_aborts(error, 0, sizeof(uint64_t));
        ft_error_destroy(error);
    }
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

static const FtProcessorVtable g_test_vtable = FT_VTABLE(FtProcessorVtable,
    .drop = test_drop,
    .process = test_process,
    .name = test_processor_name,
);

/* Construct a heap-allocated processor handle through the library. */
static void* make_processor_handle(void* user_data) {
    return ft_vtable_handle_new(FT_PROCESSOR_TYPE_TAG, &g_test_vtable, user_data);
}

void vtable_initializer_macro(void) {
    /* A sparse vtable counts all method slots, not just initialized methods. */
    typedef struct {
        uint16_t _num_slots;
        uint16_t _reserved1;
        uint32_t _reserved2;
        void (*drop)(void*);
        void (*__reserved_0)(void);
        void (*__reserved_1)(void);
        int32_t (*process)(void*, int32_t);
    } SparseVtable;
    static const SparseVtable sparse = FT_VTABLE(SparseVtable,
        .process = test_process,
    );
    static const FtProcessorVtable empty = FT_VTABLE(FtProcessorVtable, );
    static const FtEmptyTraitVtable drop_only = FT_VTABLE(FtEmptyTraitVtable,
        .drop = test_drop,
    );
    static const FtRetiredTraitVtable retired = FT_VTABLE(FtRetiredTraitVtable, );

    _Static_assert(offsetof(FtProcessorVtable, drop) == 8, "vtable header must be 8 bytes");
    _Static_assert(FT_VTABLE_LEN(FtProcessorVtable) == 2, "two virtual method slots, excluding drop");
    _Static_assert(FT_VTABLE_LEN(FtEmptyTraitVtable) == 0, "empty trait has no virtual method slots");
    _Static_assert(sizeof(FtEmptyTraitVtable) == 8 + sizeof(drop_only.drop), "empty trait contains only header and drop");
    _Static_assert(FT_VTABLE_LEN(FtRetiredTraitVtable) == 3, "retired slots and gaps still count");
    assert(g_test_vtable._num_slots == 2);
    assert(g_test_vtable._reserved1 == 0);
    assert(g_test_vtable._reserved2 == 0);
    assert(sparse._num_slots == 3);
    assert(sparse.drop == NULL);
    assert(sparse.__reserved_0 == NULL);
    assert(sparse.__reserved_1 == NULL);
    assert(sparse.process(NULL, 21) == 42);
    assert(empty._num_slots == 2);
    assert(empty.drop == NULL);
    assert(empty.process == NULL);
    assert(empty.name == NULL);
    assert(drop_only._num_slots == 0);
    assert(drop_only.drop == test_drop);
    assert(retired._num_slots == 3);
    assert(retired.__reserved_0 == NULL);
    assert(retired.__reserved_1 == NULL);
    assert(retired.__reserved_2 == NULL);
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

int main(int argc, char** argv) {
    if (argc == 2 && strcmp(argv[1], "--payload-abort-tests") == 0) {
        RUN_TEST(error_payload_invalid_access_aborts);
        return 0;
    }
    assert(argc == 1);
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
    RUN_TEST(error_factory_constructs_fieldless_variants_only);
    RUN_TEST(error_payload_uses_native_field_indices);

    printf("\n[vtable/implementable]\n");
    RUN_TEST(vtable_initializer_macro);
    RUN_TEST(vtable_dyn_dispatch_process);

    RUN_TEST(vtable_drop_callback);

    printf("\n[lifetime types]\n");
    RUN_TEST(lifetime_type_borrowing_handle);
    RUN_TEST(lifetime_type_reading_through_borrow);
    RUN_TEST(lifetime_type_str_methods);

    printf("\n[optional fd]\n");
    RUN_TEST(method_returning_result_optional_fd_some);
    RUN_TEST(method_returning_result_optional_fd_none);
    RUN_TEST(method_returning_result_optional_fd_err);

    printf("\n[value structs]\n");
    RUN_TEST(value_struct_inline_and_by_pointer);
    RUN_TEST(value_struct_inline_field_of_struct);
    RUN_TEST(value_struct_result_return);
    RUN_TEST(value_struct_optional_return);
    RUN_TEST(value_struct_optional_return_free_function);

    printf("\n[destroy]\n");
    RUN_TEST(destroy_handle);
    RUN_TEST(destroy_null_handle);

    printf("\nAll %d tests passed!\n", test_count);
    return 0;
}
