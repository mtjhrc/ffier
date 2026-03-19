#include <stdio.h>
#include "mycalculator.h"

int main(void) {
    MyCalculatorHandle calc = mycalculator_create();

    printf("add(3, 4) = %d\n", mycalculator_add(calc, 3, 4));
    printf("multiply(5, 6) = %d\n", mycalculator_multiply(calc, 5, 6));
    printf("negate(42) = %ld\n", mycalculator_negate(calc, 42));
    printf("is_positive(-1) = %d\n", mycalculator_is_positive(calc, -1));
    mycalculator_set_precision(calc, 64);

    mycalculator_destroy(calc);
    printf("All C tests passed!\n");
    return 0;
}
