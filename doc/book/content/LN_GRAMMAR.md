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

The logic block environment also tracks the time-of-day and solar
position. The latitude and longitude parameters in `.drmem.toml` are
used for the solar calculations. These parameters are available to
expressions through specially named variables:

| Variable | Description |
|----------|-------------|
| {local:second} | second (0-59) value of local time |
| {local:minute} | minute (0-59) value of local time |
| {local:hour} | hour (0-23) value of local time |
| {local:day} | day of month (1-31) value of local time |
| {local:month} | month (0-11) value of local time |
| {local:year} | year value of local time |
| {local:DOW} | day of week (0-6, 0 is Monday) value of local time |
| {local:DOY} | day of year (0-365) value of local time |
| {utc:second} | second (0-59) value of UTC time |
| {utc:minute} | minute (0-59) value of UTC time |
| {utc:hour} | hour (0-23) value of UTC time |
| {utc:day} | day of month (1-31) value of UTC time |
| {utc:month} | month (0-11) value of UTC time |
| {utc:year} | year value of UTC time |
| {utc:DOW} | day of week (0-6, 0 is Monday) value of UTC time |
| {utc:DOY} | day of year (0-365) value of UTC time |
| {solar:alt} | elevation of the sun (90 - -90 degrees.) Negative values mean the sun is below the horizon. |
| {solar:az} | azimuth of the sun |
| {solar:dec} | declination of the sun |
| {solar:ra} | right ascension of the sun |

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
