#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
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
    FtTestError err = ft_widget_validate(w);
    assert(err.code == 0);
    ft_widget_destroy(w);
}

void method_returning_result_void_err(void) {
    FtWidget w = ft_widget_new();
    FtTestError err = ft_widget_fail_always(w);
    assert(err.code == FT_TEST_ERROR_CUSTOM_MESSAGE);
    ft_test_error_free(&err);
    ft_widget_destroy(w);
}

void method_returning_result_value_ok(void) {
    FtWidget w = ft_widget_new();
    int32_t result = -1;
    FtTestError err = ft_widget_parse_count(w, FT_STR("hello"), &result);
    assert(err.code == 0);
    assert(result == 5); /* len("hello") == 5 */
    ft_widget_destroy(w);
}

void method_returning_result_value_err(void) {
    FtWidget w = ft_widget_new();
    int32_t result = -1;
    FtTestError err = ft_widget_parse_count(w, FT_STR("error"), &result);
    assert(err.code == FT_TEST_ERROR_NOT_FOUND);
    ft_test_error_free(&err);
    ft_widget_destroy(w);
}

void method_returning_result_str_ok(void) {
    FtWidget w = ft_widget_new();
    FtStr result = { 0 };
    FtTestError err = ft_widget_describe(w, 0, &result);
    assert(err.code == 0);
    assert_ft_str_eq(result, "zero");
    ft_widget_destroy(w);
}

void method_returning_result_str_err(void) {
    FtWidget w = ft_widget_new();
    FtStr result = { 0 };
    FtTestError err = ft_widget_describe(w, 99, &result);
    assert(err.code == FT_TEST_ERROR_NOT_FOUND);
    ft_test_error_free(&err);
    ft_widget_destroy(w);
}

void method_returning_result_handle_ok(void) {
    FtWidget w = ft_widget_new();
    ft_widget_set_count(w, 7);
    FtGadget g = NULL;
    FtTestError err = ft_widget_try_create_gadget(w, true, &g);
    assert(err.code == 0);
    assert(g != NULL);
    assert(ft_gadget_get(g) == 7);
    ft_gadget_destroy(g);
    ft_widget_destroy(w);
}

void method_returning_result_handle_err(void) {
    FtWidget w = ft_widget_new();
    FtGadget g = NULL;
    FtTestError err = ft_widget_try_create_gadget(w, false, &g);
    assert(err.code == FT_TEST_ERROR_NOT_FOUND);
    assert(g == NULL); /* should remain NULL on error */
    ft_test_error_free(&err);
    ft_widget_destroy(w);
}

void method_returning_result_fail_with_value(void) {
    FtWidget w = ft_widget_new();
    int32_t result = -1;
    FtTestError err = ft_widget_fail_with_value(w, &result);
    assert(err.code == FT_TEST_ERROR_INVALID_INPUT);
    ft_test_error_free(&err);
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
    FtTestError err = ft_config_validated(&c);
    assert(err.code == 0);
    assert_ft_str_eq(ft_config_get_name(c), "valid");
    ft_config_destroy(c);
}

void builder_method_returning_result_self_err(void) {
    FtConfig c = ft_config_new();
    /* name is empty — validated() should fail */
    FtTestError err = ft_config_validated(&c);
    assert(err.code == FT_TEST_ERROR_INVALID_INPUT);
    ft_test_error_free(&err);
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
    FtGizmo g = NULL;
    FtTestError err = ft_gizmo_builder_try_build(b, &g);
    /* b is consumed */
    assert(err.code == 0);
    assert(g != NULL);
    assert_ft_str_eq(ft_gizmo_name(g), "valid");
    assert(ft_gizmo_size(g) == 50);
    ft_gizmo_destroy(g);
}

void builder_consuming_self_returning_result_handle_err(void) {
    FtGizmoBuilder b = ft_gizmo_builder_new();
    /* name empty — try_build() should fail */
    FtGizmo g = NULL;
    FtTestError err = ft_gizmo_builder_try_build(b, &g);
    /* b is consumed */
    assert(err.code == FT_TEST_ERROR_INVALID_INPUT);
    assert(g == NULL);
    ft_test_error_free(&err);
}

/* ===================================================================== */
/* Error type FFI                                                        */
/* ===================================================================== */

void error_code_constants(void) {
    assert(FT_TEST_ERROR_NOT_FOUND == 1);
    assert(FT_TEST_ERROR_CUSTOM_MESSAGE == 2);
    assert(FT_TEST_ERROR_INVALID_INPUT == 3);
}

void error_message_auto_generated(void) {
    FtWidget w = ft_widget_new();
    FtTestError err = ft_widget_fail_always(w);
    assert(err.code == FT_TEST_ERROR_CUSTOM_MESSAGE);
    const char* msg = ft_test_error_message(&err);
    assert(msg != NULL);
    assert(strcmp(msg, "custom error message") == 0);
    ft_test_error_free(&err);
    ft_widget_destroy(w);
}

void error_message_not_found(void) {
    /* Trigger NotFound error — auto-humanized message "not found" */
    FtWidget w = ft_widget_new();
    int32_t result;
    FtTestError err = ft_widget_parse_count(w, FT_STR("error"), &result);
    assert(err.code == FT_TEST_ERROR_NOT_FOUND);
    const char* msg = ft_test_error_message(&err);
    assert(msg != NULL);
    assert(strcmp(msg, "not found") == 0);
    ft_test_error_free(&err);
    ft_widget_destroy(w);
}

void error_free(void) {
    FtWidget w = ft_widget_new();
    FtTestError err = ft_widget_fail_always(w);
    assert(err.code != 0);
    ft_test_error_free(&err);
    assert(err.code == 0); /* free resets to ok */
    ft_widget_destroy(w);
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

static int g_last_notify_code = -1;

static void test_on_notify(void* self_data, int32_t code) {
    (void)self_data;
    g_last_notify_code = code;
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
    .on_notify = test_on_notify,
};

static FtProcessorVtableRef g_test_vtable_ref = NULL;

static FtProcessorVtableRef get_test_vtable_ref(void) {
    if (!g_test_vtable_ref) {
        g_test_vtable_ref = ft_processor_new_vtable(&g_test_vtable, sizeof(g_test_vtable));
    }
    return g_test_vtable_ref;
}

void vtable_constructor(void) {
    void* proc = ft_processor_from_vtable(NULL, get_test_vtable_ref());
    assert(proc != NULL);
    /* Just test construction and destroy */
    ft_processor_destroy(proc);
}

void vtable_dyn_dispatch_process(void) {
    FtPipeline p = ft_pipeline_new();
    g_last_notify_code = -1;
    void* proc = ft_processor_from_vtable(NULL, get_test_vtable_ref());
    ft_pipeline_run(p, proc, 21);
    /* process(21) = 42, then on_notify(42) */
    assert(g_last_notify_code == 42);
    assert(ft_pipeline_result_count(p) == 1);
    int32_t last = -1;
    FtTestError err = ft_pipeline_last_result(p, &last);
    assert(err.code == 0);
    assert(last == 42);
    ft_pipeline_destroy(p);
}

void vtable_supertrait_method(void) {
    /* on_notify is tested via vtable_dyn_dispatch_process above.
     * Here verify it independently through a separate invocation. */
    FtPipeline p = ft_pipeline_new();
    g_last_notify_code = -1;
    void* proc = ft_processor_from_vtable(NULL, get_test_vtable_ref());
    ft_pipeline_run(p, proc, 5);
    /* process(5) = 10, on_notify(10) */
    assert(g_last_notify_code == 10);
    ft_pipeline_destroy(p);
}

void vtable_drop_callback(void) {
    g_drop_called = 0;
    void* proc = ft_processor_from_vtable(NULL, get_test_vtable_ref());
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
    RUN_TEST(error_code_constants);
    RUN_TEST(error_message_auto_generated);
    RUN_TEST(error_message_not_found);
    RUN_TEST(error_free);

    printf("\n[vtable/implementable]\n");
    RUN_TEST(vtable_dyn_dispatch_process);
    RUN_TEST(vtable_supertrait_method);
    RUN_TEST(vtable_drop_callback);

    printf("\n[lifetime types]\n");
    RUN_TEST(lifetime_type_borrowing_handle);
    RUN_TEST(lifetime_type_reading_through_borrow);
    RUN_TEST(lifetime_type_str_methods);

    printf("\n[destroy]\n");
    RUN_TEST(destroy_handle);
    RUN_TEST(destroy_null_handle);

    printf("\nAll %d tests passed!\n", test_count);
    return 0;
}
