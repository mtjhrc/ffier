#include <stdio.h>
#include <assert.h>
#include <string.h>
#include "mylib.h"

int main(int argc, char **argv) {
    MylibCalculator calc = mylib_calculator_new();

    /* Plain returns */
    printf("add(3, 4) = %d\n", mylib_calculator_add(calc, 3, 4));
    printf("multiply(5, 6) = %d\n", mylib_calculator_multiply(calc, 5, 6));
    printf("negate(42) = %ld\n", mylib_calculator_negate(calc, 42));
    printf("is_positive(-1) = %d\n", mylib_calculator_is_positive(calc, -1));
    mylib_calculator_set_precision(calc, 64);

    /* &str return (zero-copy, borrows from object) */
    MylibStr name = mylib_calculator_name(calc);
    printf("name() = %.*s\n", (int)name.len, name.data);

    /* &str param + return (zero-copy round-trip) */
    MylibStr greeting = mylib_calculator_echo(calc, MYLIB_STR("hello from C!"));
    printf("echo() = %.*s\n", (int)greeting.len, greeting.data);

    /* BorrowedFd param */
    printf("fd_number(STDOUT) = %d\n", mylib_calculator_fd_number(calc, 1));
    assert(mylib_calculator_fd_number(calc, 1) == 1);

    /* &[&str] param (string slice) */
    MylibStr parts[] = { MYLIB_STR("hello"), MYLIB_STR("from"), MYLIB_STR("C") };
    mylib_calculator_set_label(calc, parts, 3);
    MylibStr label = mylib_calculator_name(calc);
    printf("set_label() then name() = %.*s\n", (int)label.len, label.data);
    assert(label.len == 12); /* "hello-from-C" */

    /* &[u8] return */
    MylibBytes data = mylib_calculator_data(calc);
    printf("data() = %zu bytes\n", data.len);

    /* Result<i32, CalcError> — success */
    int32_t result;
    MylibCalcError err = mylib_calculator_divide(calc, 10, 3, &result);
    assert(err.code == 0);
    printf("divide(10, 3) = %d\n", result);

    /* Result<i32, CalcError> — error */
    err = mylib_calculator_divide(calc, 10, 0, &result);
    assert(err.code == MYLIB_CALC_ERROR_DIVISION_BY_ZERO);
    printf("divide(10, 0) = error %lu: %s\n", err.code, mylib_calc_error_message(&err));
    mylib_calc_error_free(&err);

    /* checked_add — overflow */
    err = mylib_calculator_checked_add(calc, 2147483647, 1, &result);
    assert(err.code == MYLIB_CALC_ERROR_OVERFLOW);
    printf("checked_add(INT_MAX, 1) = error %lu: %s\n", err.code, mylib_calc_error_message(&err));
    mylib_calc_error_free(&err);

    /* Result<&str, CalcError> — success */
    MylibStr desc;
    err = mylib_calculator_describe(calc, 1, &desc);
    assert(err.code == 0);
    printf("describe(1) = %.*s\n", (int)desc.len, desc.data);

    /* Result<&str, CalcError> — error */
    err = mylib_calculator_describe(calc, 99, &desc);
    assert(err.code == MYLIB_CALC_ERROR_OVERFLOW);
    printf("describe(99) = error %lu: %s\n", err.code, mylib_calc_error_message(&err));
    mylib_calc_error_free(&err);

    /* Result<(), CalcError> */
    err = mylib_calculator_validate(calc);
    assert(err.code == 0);
    printf("validate() = ok\n");

    /* Returning an exported object (new handle) */
    MylibCalcResult res = mylib_calculator_create_result(calc);
    printf("create_result() value = %d\n", mylib_calc_result_get(res));

    /* Passing &mut ExportedType (mutable borrow of handle) */
    mylib_calculator_accumulate(calc, res, 10);
    mylib_calculator_accumulate(calc, res, 20);
    printf("after accumulate(10, 20) = %d\n", mylib_calc_result_get(res));

    /* Passing &ExportedType (immutable borrow of handle) */
    printf("read_result() = %d\n", mylib_calculator_read_result(calc, res));
    mylib_calc_result_destroy(res);

    /* Result<ExportedType, Error> — success */
    MylibCalcResult res2;
    err = mylib_calculator_try_create_result(calc, 10, 2, &res2);
    assert(err.code == 0);
    printf("try_create_result(10, 2) = %d\n", mylib_calc_result_get(res2));
    mylib_calc_result_destroy(res2);

    /* Result<ExportedType, Error> — error */
    err = mylib_calculator_try_create_result(calc, 10, 0, &res2);
    assert(err.code == MYLIB_CALC_ERROR_DIVISION_BY_ZERO);
    printf("try_create_result(10, 0) = error %lu: %s\n", err.code, mylib_calc_error_message(&err));
    mylib_calc_error_free(&err);

    mylib_calculator_destroy(calc);
    printf("All C tests passed!\n");

    /* Test RTTI: pass a CalcResult handle where a Calculator handle is expected.
     * This should panic with a clear type mismatch message. */
    if (argc > 1 && strcmp(argv[1], "--test-rtti") == 0) {
        MylibCalculator calc2 = mylib_calculator_new();
        MylibCalcResult bad = mylib_calculator_create_result(calc2);
        /* Passing a CalcResult handle where Calculator is expected → panic */
        printf("Calling mylib_calculator_add with wrong handle type (will abort)...\n");
        fflush(stdout);
        mylib_calculator_add((MylibCalculator)bad, 1, 2);
        /* unreachable */
        mylib_calculator_destroy(calc2);
    }

    return 0;
}
