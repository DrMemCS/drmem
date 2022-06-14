# drmem-drv-cycle

Provides devices that implement a square wave output. Anytime the
`enable` device transitions from `false` to `true`, the output starts
to alternate between `true` and `false` at a configured frequency.

This driver is always available in DrMem.

## Configuration

This driver uses the following configuration parameters.

- `millis` is the number of milliseconds for the full cycle.
- `enabled` is an optional boolean value which, when `true`, will set
  the `enable` device's initial value to `true`. If not provided, it
  defaults to `false`.

## Devices

The driver creates these devices:

| Base Name | Type     | Units | Comment                                                |
|-----------|----------|-------|--------------------------------------------------------|
| `enable`  | bool, RW |       | A `false` to `true` transition will start the cycling. |
| `output`  | bool, RO |       | Output state.                                          |

Every value sent to the `enable` device will be reported -- even
duplicates. This allows one to, if using the redis backend, see the
history of settings made to the device. The `output` device, however,
only reports state changes. So if a client sends multiple `true`
values, the `output` would only report the state changes.

## History

Added in v0.1.0.
