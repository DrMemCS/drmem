# drmem-drv-integrate

Implements a floating point accumulator that sums every value applied to the `data` device. The `output` device reports the latest sum as a floating point value. `data` accepts floating point values. Boolean values are converted to `0` and `1`. When the `reset` device is set to `true`, the sum is reset to `0.0`.

This driver is always available in DrMem.

## Configuration

This driver has no configuration parameters.

## Devices

The driver creates these devices:

| Base Name | Type     | Units | Comment                                                |
|-----------|----------|-------|--------------------------------------------------------|
| `data`  | float, RW |       | Value is added to sum. |
| `reset`  | bool, RW |       | A `false` to `true` transition resets the sum to 0. |
| `output`  | float, RO    |       | The current sum. |

## History

Added in v0.8.0.
