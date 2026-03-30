#include <stdio.h>
#include <assert.h>
#include <unistd.h>
#include "mylib.h"

int main(void) {
    /* ---- Calculator ---- */
    MylibCalculator calc = mylib_calculator_new();

    printf("add(3, 4) = %d\n", mylib_calculator_add(calc, 3, 4));
    printf("is_positive(-1) = %d\n", mylib_calculator_is_positive(calc, -1));

    /* Result<i32, E> — ok + error */
    int32_t quotient;
    MylibCalcError err = mylib_calculator_divide(calc, 10, 3, &quotient);
    assert(err.code == 0);
    printf("divide(10, 3) = %d\n", quotient);

    err = mylib_calculator_divide(calc, 1, 0, &quotient);
    assert(err.code == MYLIB_CALC_ERROR_DIVISION_BY_ZERO);
    mylib_calc_error_free(&err);

    /* Handle return + &mut Handle / &Handle params */
    MylibCalcResult res = mylib_calculator_create_result(calc);
    mylib_calculator_accumulate(calc, res, 10);
    mylib_calculator_accumulate(calc, res, 20);
    printf("accumulated = %d\n", mylib_calculator_read_result(calc, res));
    mylib_calc_result_destroy(res);

    /* Result<Handle, E> */
    MylibCalcResult res2;
    err = mylib_calculator_try_divide_result(calc, 10, 2, &res2);
    assert(err.code == 0);
    printf("try_divide_result(10, 2) = %d\n", mylib_calc_result_get(res2));
    mylib_calc_result_destroy(res2);

    mylib_calculator_destroy(calc);

    /* ---- TextBuffer ---- */
    MylibTextBuffer buf = mylib_text_buffer_new(dup(1));
    assert(mylib_text_buffer_fd(buf) > 0);

    mylib_text_buffer_write(buf, MYLIB_STR("hello "));
    MylibStr parts[] = { MYLIB_STR("wo"), MYLIB_STR("rld") };
    mylib_text_buffer_write_parts(buf, parts, 2);

    MylibStr contents = mylib_text_buffer_contents(buf);
    printf("buffer = %.*s\n", (int)contents.len, contents.data);

    MylibBytes raw = mylib_text_buffer_as_bytes(buf);
    assert(raw.len == 11);

    /* flush() writes to the owned fd */
    fflush(stdout);
    MylibBufferError buf_err = mylib_text_buffer_flush(buf);
    assert(buf_err.code == 0);
    printf("\n");

    mylib_text_buffer_clear(buf);
    mylib_text_buffer_destroy(buf);

    printf("All C tests passed!\n");
    return 0;
}
