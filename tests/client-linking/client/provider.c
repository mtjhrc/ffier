#ifdef _WIN32
#include <windows.h>
#define EXPORT __declspec(dllexport)
#else
#include <unistd.h>
#define EXPORT
#endif

#ifdef OTHER_LIBRARY
EXPORT int fixture_answer(void) { return 99; }
EXPORT int fixture_other(void) { return 77; }
EXPORT int fixture_missing(void) { return -1; }
#else
EXPORT int fixture_answer(void) { return 42; }
EXPORT int fixture_other(void) { return 11; }
/* The older provider intentionally has no fixture_missing symbol. */
#endif

static int destroyed_widgets;
EXPORT void fixture_widget_destroy(void *handle) { (void)handle; ++destroyed_widgets; }
EXPORT int fixture_destroy_count(void) { return destroyed_widgets; }

/* Concrete Widget handles can transfer without the missing callback allocator. */
#ifdef _WIN32
EXPORT void fixture_consume_fd_callback(void *fd, void *widget) {
    CloseHandle(fd);
#else
EXPORT void fixture_consume_fd_callback(int fd, void *widget) {
    close(fd);
#endif
    fixture_widget_destroy(widget);
}

EXPORT int fixture_echo_args(int first, int second) { return first + second; }
