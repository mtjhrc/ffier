#ifndef FFIER_FT_H
#define FFIER_FT_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>

typedef void* FtTestError;
typedef void* FtWidget;
typedef void* FtGadget;
typedef void* FtConfig;
typedef void* FtGizmo;
typedef void* FtGizmoBuilder;
typedef void* FtView;
typedef void* FtViewFactory;
typedef void* FtPipeline;
typedef void* FtApple;
typedef void* FtOrange;
typedef void* FtBanana;
typedef void* FtMango;
typedef void* FtPeach;
typedef void* FtPlum;
typedef void* FtGrape;
typedef void* FtLemon;
typedef void* FtMixer;
typedef void* FtSprocket;
typedef void* FtAttachment; /* FtSprocket */
typedef void* FtError; /* FtTestError | FtVtableError */
typedef void* FtFruit; /* FtApple | FtOrange | FtBanana | FtMango | FtPeach | FtPlum | FtGrape | FtLemon | FtVtableFruit */
typedef void* FtProcessor; /* FtVtableProcessor */
typedef void* FtPushStr; /* FtVtablePushStr */
typedef void* FtSnapshot; /* FtView | FtWidget | FtGadget */
typedef void* FtWeighable; /* FtApple | FtVtableWeighable */

typedef uint64_t FtResult;
#define FT_RESULT_SUCCESS 0

/* Caller must ensure data is valid UTF-8 */
typedef struct {
    const char* data;
    size_t len;
} FtStr;

typedef struct {
    const uint8_t* data;
    size_t len;
} FtBytes;

#define FT_STR(s) ((FtStr){ .data = (s), .len = (s) ? strlen(s) : 0 })
#if defined(__GNUC__)
#define FT_BYTES(arr) ({ \
    _Static_assert( \
        !__builtin_types_compatible_p(typeof(arr), typeof(&(arr)[0])), \
        "FT_BYTES() requires an array, not a pointer"); \
    ((FtBytes){ .data = (const uint8_t*)(arr), .len = sizeof(arr) }); \
})
#else
#define FT_BYTES(arr) \
    ((FtBytes){ .data = (const uint8_t*)(arr), .len = sizeof(arr) })
#endif

/* Free an owned string returned by the library */
void ft_str_free(FtStr s);

/**
 * Stack-allocated temporary handle for passing vtable-based objects.
 * Only valid for the duration of the call — the callee borrows, not owns.
 */
typedef struct {
    uint32_t type_tag;
    uint32_t metadata;
    const void *vtable_ptr;
    const void *user_data;
    uint16_t vtable_size;
} FtVtableHandle;

#define FT_VTABLE_HANDLE(tag, vtable, self_data) \
    ((FtVtableHandle){ .type_tag = (tag), .metadata = 0, \
      .vtable_ptr = &(vtable), .user_data = (self_data), \
      .vtable_size = sizeof(vtable) })


/* LogLevel ---------------------------------------------------------- */

#define FT_LOG_LEVEL_OFF 0
#define FT_LOG_LEVEL_ERROR 1
#define FT_LOG_LEVEL_WARN 2
#define FT_LOG_LEVEL_INFO 3
#define FT_LOG_LEVEL_DEBUG 4
#define FT_LOG_LEVEL_TRACE 5

/* Permissions ------------------------------------------------------- */

#define FT_PERMISSIONS_READ 1
#define FT_PERMISSIONS_WRITE 2
#define FT_PERMISSIONS_EXECUTE 4
#define FT_PERMISSIONS_DELETE 8

/* TestError --------------------------------------------------------- */

#define FT_ERROR_TEST_NOT_FOUND ((uint64_t)1 << 32 | 1)
#define FT_ERROR_TEST_CUSTOM_MESSAGE ((uint64_t)1 << 32 | 2)
#define FT_ERROR_TEST_INVALID_INPUT ((uint64_t)1 << 32 | 3)

/* Widget ------------------------------------------------------------ */

/** Create a new widget with default values. */
FtWidget ft_widget_new();
/** Create a widget with a given name. */
FtWidget ft_widget_with_name(FtStr name);
/** Get the current count. */
int32_t ft_widget_get_count(FtWidget handle);
/** Set the count. */
void ft_widget_set_count(FtWidget handle, int32_t n);
/** Set count and return `&mut Self` for method chaining. */
void ft_widget_with_count(FtWidget handle, int32_t n);
/** Get the widget name. */
FtStr ft_widget_name(FtWidget handle);
/** Get the raw name bytes. */
FtBytes ft_widget_data(FtWidget handle);
/** Sum the bytes of a byte slice. */
int32_t ft_widget_sum_bytes(FtWidget handle, FtBytes data);
/** Echo back the given string (zero-copy borrow passthrough). */
FtStr ft_widget_echo(FtWidget handle, FtStr s);
/** Check if the widget is active. */
bool ft_widget_is_active(FtWidget handle);
/** Negate a 64-bit integer. */
int64_t ft_widget_negate(FtWidget handle, int64_t v);
/** Validate internal state (always succeeds for default widget). */
FtResult ft_widget_validate(FtWidget handle, FtError* err_out);
/**
 * Parse a count value from the name length, returning error if name matches trigger.
 *
 * # Arguments
 *
 * - `s`: the input string whose length becomes the count.
 *
 * # Returns
 *
 * The count derived from the name length.
 */
FtResult ft_widget_parse_count(FtWidget handle, FtStr s, int32_t* result, FtError* err_out);
/**
 * Describe a code as a string.
 *
 * # Arguments
 *
 * * `code` - the numeric code to look up.
 */
FtResult ft_widget_describe(FtWidget handle, int32_t code, FtStr* result, FtError* err_out);
/** Always fails with an error. */
FtResult ft_widget_fail_always(FtWidget handle, FtError* err_out);
/** Always fails with an error (value variant). */
FtResult ft_widget_fail_with_value(FtWidget handle, int32_t* result, FtError* err_out);
/** Set tags from a string slice. */
void ft_widget_set_tags(FtWidget handle, const FtStr* tags, size_t tags_len);
/** Get joined tags. */
FtStr ft_widget_tags_joined(FtWidget handle);
/** Create a new gadget with the widget's count as initial value. */
FtGadget ft_widget_create_gadget(FtWidget handle);
/** Try to create a gadget; fails if ok is false. */
FtGadget ft_widget_try_create_gadget(FtWidget handle, bool ok, FtError* err_out);
/** Read a gadget's value. */
int32_t ft_widget_read_gadget(FtWidget handle, FtGadget g);
/** Update a gadget's value. */
void ft_widget_update_gadget(FtWidget handle, FtGadget g, int32_t v);
/** Set the name, or reset to default if `None`. */
void ft_widget_set_name(FtWidget handle, FtStr name);
/** Get an owned copy of the name. */
FtStr ft_widget_owned_name(FtWidget handle);
/** Add a permission flag to the widget's permissions and return the result. */
uint32_t ft_widget_add_permission(FtWidget handle, uint32_t base, uint32_t flag);
/** Consume the widget (by-value self, void return). */
void ft_widget_consume(FtWidget handle);
/** Get the raw fd number from a borrowed fd. */
int32_t ft_widget_fd_number(FtWidget handle, int fd);
/** Get the raw fd number, or -1 if None. */
int32_t ft_widget_fd_number_optional(FtWidget handle, int fd);
/**
 * Maybe return a borrowed fd depending on `selector`:
 * < 0 → error, 0 → Ok(None), > 0 → Ok(Some(stdin)).
 */
FtResult ft_widget_maybe_fd(FtWidget handle, int32_t selector, int* result, FtError* err_out);
/** Duplicate a file descriptor (returns owned fd). */
int ft_widget_dup_fd(FtWidget handle, int fd);
void ft_widget_destroy(FtWidget handle);

/* Gadget ------------------------------------------------------------ */

/** Get the gadget value. */
int32_t ft_gadget_get(FtGadget handle);
void ft_gadget_destroy(FtGadget handle);

/* Config ------------------------------------------------------------ */

/** Create a new config. */
FtConfig ft_config_new();
/** Set the name (builder pattern: consumes self, returns Self). */
void ft_config_set_name(FtConfig* handle, FtStr name);
/** Set the size (builder pattern). */
void ft_config_set_size(FtConfig* handle, int32_t size);
/** Validate and return self, or error if name is empty. */
FtResult ft_config_validated(FtConfig* handle, FtError* err_out);
/** Get the config name. */
FtStr ft_config_get_name(FtConfig handle);
/** Get the config size. */
int32_t ft_config_get_size(FtConfig handle);
void ft_config_destroy(FtConfig handle);

/* Gizmo ------------------------------------------------------------- */

/** Get the gizmo name. */
FtStr ft_gizmo_name(FtGizmo handle);
/** Get the gizmo size. */
int32_t ft_gizmo_size(FtGizmo handle);
void ft_gizmo_destroy(FtGizmo handle);

/* GizmoBuilder ------------------------------------------------------ */

/** Create a new gizmo builder. */
FtGizmoBuilder ft_gizmo_builder_new();
/** Set the gizmo name. */
void ft_gizmo_builder_set_name(FtGizmoBuilder handle, FtStr name);
/** Set the gizmo size. */
void ft_gizmo_builder_set_size(FtGizmoBuilder handle, int32_t size);
/** Build the gizmo (consumes builder, returns different type). */
FtGizmo ft_gizmo_builder_build(FtGizmoBuilder handle);
/** Try to build the gizmo; fails if name is empty. */
FtGizmo ft_gizmo_builder_try_build(FtGizmoBuilder handle, FtError* err_out);
void ft_gizmo_builder_destroy(FtGizmoBuilder handle);

/* View -------------------------------------------------------------- */

/** Create a view that borrows a widget. */
FtView ft_view_create(FtWidget source);
/**
 * Create a view with a custom label.
 *
 * Takes two reference params so lifetime elision can't resolve `'_`
 * in the return type — the struct lifetime must be preserved explicitly.
 */
FtView ft_view_create_labeled(FtWidget source, FtStr label);
/** Read the source widget's count through the borrow. */
int32_t ft_view_source_count(FtView handle);
/** Set the view label. */
void ft_view_set_label(FtView handle, FtStr label);
/** Get the view label. */
FtStr ft_view_label(FtView handle);
/** Copy label from another snapshot (tests impl Trait auto-dispatch). */
void ft_view_copy_label(FtView handle, FtSnapshot other);
void ft_view_destroy(FtView handle);

/* ViewFactory ------------------------------------------------------- */

FtViewFactory ft_view_factory_new();
/**
 * Create a view from a source widget with a label.
 *
 * Multiple reference params + lifetime-parameterized return type forces
 * the generator to introduce a method-level lifetime (can't elide).
 */
FtView ft_view_factory_create_view(FtWidget source, FtStr label);
void ft_view_factory_destroy(FtViewFactory handle);

/* Pipeline ---------------------------------------------------------- */

/** Create a new pipeline. */
FtPipeline ft_pipeline_new();
/** Run a processor on the given input. */
void ft_pipeline_run(FtPipeline handle, FtProcessor proc, int32_t input);
/** Get the number of results. */
int32_t ft_pipeline_result_count(FtPipeline handle);
/** Get the last result, or error if empty. */
FtResult ft_pipeline_last_result(FtPipeline handle, int32_t* result, FtError* err_out);
void ft_pipeline_destroy(FtPipeline handle);

/* Apple ------------------------------------------------------------- */

FtApple ft_apple_new(int32_t weight);
void ft_apple_destroy(FtApple handle);

/* Orange ------------------------------------------------------------ */

FtOrange ft_orange_new(int32_t juice);
void ft_orange_destroy(FtOrange handle);

/* Banana ------------------------------------------------------------ */

FtBanana ft_banana_new(int32_t v);
void ft_banana_destroy(FtBanana handle);

/* Mango ------------------------------------------------------------- */

FtMango ft_mango_new(int32_t v);
void ft_mango_destroy(FtMango handle);

/* Peach ------------------------------------------------------------- */

FtPeach ft_peach_new(int32_t v);
void ft_peach_destroy(FtPeach handle);

/* Plum -------------------------------------------------------------- */

FtPlum ft_plum_new(int32_t v);
void ft_plum_destroy(FtPlum handle);

/* Grape ------------------------------------------------------------- */

FtGrape ft_grape_new(int32_t v);
void ft_grape_destroy(FtGrape handle);

/* Lemon ------------------------------------------------------------- */

FtLemon ft_lemon_new(int32_t v);
void ft_lemon_destroy(FtLemon handle);

/* Mixer ------------------------------------------------------------- */

FtMixer ft_mixer_new();
void ft_mixer_add(FtMixer* handle, FtFruit fruit);
/**
 * Returns the length of a fruit's label. Used to test that vtable
 * default method detection works for custom client types crossing FFI.
 */
int32_t ft_mixer_fruit_label_len(FtMixer handle, FtFruit fruit);
/** Both concrete (9^2=81 > 64, override with annotation). */
int32_t ft_mixer_blend_concrete(FtMixer handle, FtFruit a, FtFruit b);
/** First concrete, second vtable (hybrid: 9+9=18 branches). */
int32_t ft_mixer_blend_hybrid(FtMixer handle, FtFruit a, FtFruit b);
/** Both vtable (9+9=18 branches). */
int32_t ft_mixer_blend_dynamic(FtMixer handle, FtFruit a, FtFruit b);
int32_t ft_mixer_total(FtMixer handle);
void ft_mixer_destroy(FtMixer handle);

/* Sprocket ---------------------------------------------------------- */

FtSprocket ft_sprocket_new(FtStr name);
void ft_sprocket_destroy(FtSprocket handle);

/* FtProcessorVtable ------------------------------------------------- */

#define FT_PROCESSOR_TYPE_TAG 10

typedef struct {
    void (*drop)(void* self_data);
    int32_t (*process)(void* self_data, int32_t input);
    FtStr (*name)(void* self_data);
} FtProcessorVtable;

/* Processor (dispatch) ---------------------------------------------- */

int32_t ft_processor_process(FtProcessor handle, int32_t input);
FtStr ft_processor_name(FtProcessor handle);
void ft_processor_destroy(FtProcessor handle);

/* FtFruitVtable ----------------------------------------------------- */

#define FT_FRUIT_TYPE_TAG 20

typedef struct {
    void (*drop)(void* self_data);
    int32_t (*value)(void* self_data);
    FtStr (*label)(void* self_data);
    FtResult (*try_count)(void* self_data, int32_t input, int32_t* result, FtError* err_out);
    int32_t (*count_tags)(void* self_data, const FtStr* tags, size_t tags_len);
} FtFruitVtable;

/* Fruit (dispatch) -------------------------------------------------- */

int32_t ft_fruit_value(FtFruit handle);
FtStr ft_fruit_label(FtFruit handle);
FtResult ft_fruit_try_count(FtFruit handle, int32_t input, int32_t* result, FtError* err_out);
int32_t ft_fruit_count_tags(FtFruit handle, const FtStr* tags, size_t tags_len);
void ft_fruit_destroy(FtFruit handle);

/* FtWeighableVtable ------------------------------------------------- */

#define FT_WEIGHABLE_TYPE_TAG 23

typedef struct {
    void (*drop)(void* self_data);
    int32_t (*weight_grams)(void* self_data);
} FtWeighableVtable;

/* Weighable (dispatch) ---------------------------------------------- */

int32_t ft_weighable_weight_grams(FtWeighable handle);
void ft_weighable_destroy(FtWeighable handle);

/* FtPushStrVtable --------------------------------------------------- */

#define FT_PUSH_STR_TYPE_TAG 24

typedef struct {
    void (*drop)(void* self_data);
    bool (*push)(void* self_data, FtStr s);
} FtPushStrVtable;

/* PushStr (dispatch) ------------------------------------------------ */

bool ft_push_str_push(FtPushStr handle, FtStr s);
void ft_push_str_destroy(FtPushStr handle);

/* FtErrorVtable ----------------------------------------------------- */

#define FT_ERROR_TYPE_TAG 25

typedef struct {
    void (*drop)(void* self_data);
    uint32_t (*code)(void* self_data);
    void (*message)(void* self_data, FtPushStr writer);
    uint64_t (*result)(void* self_data);
} FtErrorVtable;

/* Error (dispatch) -------------------------------------------------- */

uint32_t ft_error_code(FtError handle);
void ft_error_message(FtError handle, FtPushStr writer);
uint64_t ft_error_result(FtError handle);
void ft_error_destroy(FtError handle);
int32_t ft_apple_value(FtApple handle);
FtResult ft_apple_try_count(FtApple handle, int32_t input, int32_t* result, FtError* err_out);
int32_t ft_apple_count_tags(FtApple handle, const FtStr* tags, size_t tags_len);
int32_t ft_orange_value(FtOrange handle);
FtResult ft_orange_try_count(FtOrange handle, int32_t input, int32_t* result, FtError* err_out);
int32_t ft_orange_count_tags(FtOrange handle, const FtStr* tags, size_t tags_len);
int32_t ft_banana_value(FtBanana handle);
FtResult ft_banana_try_count(FtBanana handle, int32_t input, int32_t* result, FtError* err_out);
int32_t ft_banana_count_tags(FtBanana handle, const FtStr* tags, size_t tags_len);
int32_t ft_mango_value(FtMango handle);
FtResult ft_mango_try_count(FtMango handle, int32_t input, int32_t* result, FtError* err_out);
int32_t ft_mango_count_tags(FtMango handle, const FtStr* tags, size_t tags_len);
int32_t ft_peach_value(FtPeach handle);
FtResult ft_peach_try_count(FtPeach handle, int32_t input, int32_t* result, FtError* err_out);
int32_t ft_peach_count_tags(FtPeach handle, const FtStr* tags, size_t tags_len);
int32_t ft_plum_value(FtPlum handle);
FtResult ft_plum_try_count(FtPlum handle, int32_t input, int32_t* result, FtError* err_out);
int32_t ft_plum_count_tags(FtPlum handle, const FtStr* tags, size_t tags_len);
int32_t ft_grape_value(FtGrape handle);
FtResult ft_grape_try_count(FtGrape handle, int32_t input, int32_t* result, FtError* err_out);
int32_t ft_grape_count_tags(FtGrape handle, const FtStr* tags, size_t tags_len);
int32_t ft_lemon_value(FtLemon handle);
FtResult ft_lemon_try_count(FtLemon handle, int32_t input, int32_t* result, FtError* err_out);
int32_t ft_lemon_count_tags(FtLemon handle, const FtStr* tags, size_t tags_len);
FtStr ft_sprocket_label(FtSprocket handle);
FtStr ft_view_snap_description(FtView handle);
int32_t ft_view_snap_source_count(FtView handle);
FtStr ft_widget_snap_description(FtWidget handle);
int32_t ft_widget_snap_source_count(FtWidget handle);
FtStr ft_gadget_snap_description(FtGadget handle);
int32_t ft_gadget_snap_source_count(FtGadget handle);
int32_t ft_apple_weight_grams(FtApple handle);
uint32_t ft_test_error_code(FtTestError handle);
void ft_test_error_message(FtTestError handle, FtPushStr writer);

/* Free functions ---------------------------------------------------- */

/** Describe a log level as a string. */
FtStr ft_log_level_name(uint32_t level);
/** Check if a log level is enabled (everything above Off). */
bool ft_log_level_is_enabled(uint32_t level);
/** Duplicate a file descriptor. */
FtResult ft_clone_fd(int fd, int* result, FtError* err_out);
FtStr ft_result_name(FtResult r);
const char* ft_result_name_cstr(FtResult r);

#endif /* FFIER_FT_H */
