#ifdef OTHER_LIBRARY
int fixture_answer(void) { return 99; }
int fixture_other(void) { return 77; }
int fixture_missing(void) { return -1; }
#else
int fixture_answer(void) { return 42; }
int fixture_other(void) { return 11; }
/* The older provider intentionally has no fixture_missing symbol. */
#endif

static int destroyed_widgets;
void fixture_widget_destroy(void *handle) { (void)handle; ++destroyed_widgets; }
int fixture_destroy_count(void) { return destroyed_widgets; }

/* Concrete Widget handles can transfer without the missing callback allocator. */
#include <unistd.h>
void fixture_consume_fd_callback(int fd, void *widget) {
    close(fd);
    fixture_widget_destroy(widget);
}

int fixture_echo_args(int first, int second) { return first + second; }
