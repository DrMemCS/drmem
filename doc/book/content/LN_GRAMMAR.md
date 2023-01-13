# Logic Node Programming Guide

All expressions have the form `EXPRESSION -> DEVICE`, where EXPRESSION is a logic expression which is described later in this document and DEVICE is a name found in the `outputs` field of the configuration.

An expression can have the following primitives:

| Form | Description |
|------|-------------|
| {var} | Uses the device associated with the key `var` in the `inputs` map |
| `true`, `false` | Boolean values |
| -2^32 .. 2^32 - 1 | 32-bit integers |
| #.### | 64-bit floating point (no +/-inf or NaN) |
| "string" | Text |

Expressions have the following functions and operators:

| Expression | Description |
|------------|-------------|
| not EXPR | Complements the boolean EXPR |
| EXPR or EXPR | Performs boolean OR on two boolean EXPRs |
| EXPR and EXPR | Performs boolean AND on two boolean EXPRs |
| EXPR = EXPR | Returns equality between EXPRs as boolean |
| EXPR <> EXPR | Returns inequality between EXPRs as boolean |
| EXPR < EXPR | Returns "less than" between EXPRs as boolean |
| EXPR <= EXPR | Returns "less than or equal" between EXPRs as boolean |
| EXPR > EXPR | Returns "greater than" between EXPRs as boolean |
| EXPR >= EXPR | Returns "greater than or equal" between EXPRs as boolean |
| EXPR + EXPR | Adds two expressions together |
| EXPR - EXPR | Subtracts two expressions |
| EXPR * EXPR | Multiplies two expressions together |
| EXPR / EXPR | Divides two expressions |
| EXPR % EXPR | Computes remainder after dividing two expressions |
