# drmem-drv-counter

Implements a counter that increments when the `increment` input transitions from `false` to `true`. A `reset` input will set the count to 0 when `true`.

This driver is always available in DrMem.

## Configuration

This driver has no configuration parameters.

## Devices

The driver creates these devices:

| Base Name | Type     | Units | Comment                                                |
|-----------|----------|-------|--------------------------------------------------------|
| `increment`  | bool, RW |       | A `false` to `true` transition will increment the count. |
| `reset`  | bool, RW |       | Setting to `true` will reset the count to 0. |
| `count`  | int, RO    |       | The current count. |

## History

Added in v0.7.0.
