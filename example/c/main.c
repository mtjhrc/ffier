#include <stdio.h>
#include <assert.h>
#include <string.h>
#include "mycalculator.h"

int main(int argc, char **argv) {
    ExMyCalculatorHandle calc = ex_mycalculator_new();

    /* Plain returns */
    printf("add(3, 4) = %d\n", ex_mycalculator_add(calc, 3, 4));
    printf("multiply(5, 6) = %d\n", ex_mycalculator_multiply(calc, 5, 6));
    printf("negate(42) = %ld\n", ex_mycalculator_negate(calc, 42));
    printf("is_positive(-1) = %d\n", ex_mycalculator_is_positive(calc, -1));
    ex_mycalculator_set_precision(calc, 64);

    /* &str return (zero-copy, borrows from object) */
    ExStr name = ex_mycalculator_name(calc);
    printf("name() = %.*s\n", (int)name.len, name.data);

    /* &str param + return (zero-copy round-trip) */
    ExStr greeting = ex_mycalculator_echo(calc, EX_STR("hello from C!"));
    printf("echo() = %.*s\n", (int)greeting.len, greeting.data);

    /* &[u8] return */
    ExBytes data = ex_mycalculator_data(calc);
    printf("data() = %zu bytes\n", data.len);

    /* Result<i32, CalcError> — success */
    int32_t result;
    ExCalcError err = ex_mycalculator_divide(calc, 10, 3, &result);
    assert(err.code == 0);
    printf("divide(10, 3) = %d\n", result);

    /* Result<i32, CalcError> — error */
    err = ex_mycalculator_divide(calc, 10, 0, &result);
    assert(err.code == EX_CALC_ERROR_DIVISION_BY_ZERO);
    printf("divide(10, 0) = error %lu: %s\n", err.code, ex_calc_error_message(&err));
    ex_calc_error_free(&err);

    /* checked_add — overflow */
    err = ex_mycalculator_checked_add(calc, 2147483647, 1, &result);
    assert(err.code == EX_CALC_ERROR_OVERFLOW);
    printf("checked_add(INT_MAX, 1) = error %lu: %s\n", err.code, ex_calc_error_message(&err));
    ex_calc_error_free(&err);

    /* Result<&str, CalcError> — success */
    ExStr desc;
    err = ex_mycalculator_describe(calc, 1, &desc);
    assert(err.code == 0);
    printf("describe(1) = %.*s\n", (int)desc.len, desc.data);

    /* Result<&str, CalcError> — error */
    err = ex_mycalculator_describe(calc, 99, &desc);
    assert(err.code == EX_CALC_ERROR_OVERFLOW);
    printf("describe(99) = error %lu: %s\n", err.code, ex_calc_error_message(&err));
    ex_calc_error_free(&err);

    /* Result<(), CalcError> */
    err = ex_mycalculator_validate(calc);
    assert(err.code == 0);
    printf("validate() = ok\n");

    /* Returning an exported object (new handle) */
    ExCalcResultHandle res = ex_mycalculator_create_result(calc);
    printf("create_result() value = %d\n", ex_calcresult_get(res));

    /* Passing &mut ExportedType (mutable borrow of handle) */
    ex_mycalculator_accumulate(calc, res, 10);
    ex_mycalculator_accumulate(calc, res, 20);
    printf("after accumulate(10, 20) = %d\n", ex_calcresult_get(res));

    /* Passing &ExportedType (immutable borrow of handle) */
    printf("read_result() = %d\n", ex_mycalculator_read_result(calc, res));
    ex_calcresult_destroy(res);

    /* Result<ExportedType, Error> — success */
    ExCalcResultHandle res2;
    err = ex_mycalculator_try_create_result(calc, 10, 2, &res2);
    assert(err.code == 0);
    printf("try_create_result(10, 2) = %d\n", ex_calcresult_get(res2));
    ex_calcresult_destroy(res2);

    /* Result<ExportedType, Error> — error */
    err = ex_mycalculator_try_create_result(calc, 10, 0, &res2);
    assert(err.code == EX_CALC_ERROR_DIVISION_BY_ZERO);
    printf("try_create_result(10, 0) = error %lu: %s\n", err.code, ex_calc_error_message(&err));
    ex_calc_error_free(&err);

    ex_mycalculator_destroy(calc);
    printf("All C tests passed!\n");

    /* Test RTTI: pass a CalcResult handle where a MyCalculator handle is expected.
     * This should panic with a clear type mismatch message. */
    if (argc > 1 && strcmp(argv[1], "--test-rtti") == 0) {
        ExMyCalculatorHandle calc2 = ex_mycalculator_new();
        ExCalcResultHandle bad = ex_mycalculator_create_result(calc2);
        /* Passing a CalcResult handle where MyCalculator is expected → panic */
        printf("Calling ex_mycalculator_add with wrong handle type (will abort)...\n");
        fflush(stdout);
        ex_mycalculator_add((ExMyCalculatorHandle)bad, 1, 2);
        /* unreachable */
        ex_mycalculator_destroy(calc2);
    }

    return 0;
}
