# drmem-drv-cycle

Provides devices that implement a repeating sequence of values.
Anytime the `enable` device transitions from `false` to `true`, the
output cycles through a list of values at a configured rate.

This driver is always available in DrMem.

## Configuration

This driver uses the following configuration parameters.

- `disabled` is the value of the `output` when the driver is inactive.
- `enabled` is an array of values to cycle through when the driver is
  enabled. Each value is assigned to the `output` device. When the
  last value of the array is used, the driver starts over at the
  beginning. The only way to stop it is to set the `enable` device to
  `false`.
- `enabled_at_boot` is an optional boolean value which, when `true`,
  will set the `enable` device's initial value to `true` and,
  therefore, start the cycling at boot time. If not provided, it
  defaults to `false`.
- `millis` is the number of milliseconds that the `output` will hold
  each value.

## Devices

The driver creates these devices:

| Base Name | Type     | Units | Comment                                                |
|-----------|----------|-------|--------------------------------------------------------|
| `enable`  | bool, RW |       | A `false` to `true` transition will start the cycling. |
| `output`  | T, RO    |       | Output state.                                          |

Every value sent to the `enable` device will be reported -- even
duplicates. This allows one to, if using the redis backend, see the
history of settings made to the device. The `output` device, however,
only reports state changes.

### Examples

In the configuration, setting `inactive` to `false` and `active` to
`[true, false]` results in a square wave where the full cycle is `2 *
millis` and the waveform has a 50% duty cycle. If you want a 25% on,
75% off duty cycle waveform, set `active` to `[true, false, false,
false]` and set `millis` to 1/4 of the full cycle.

## History

Added in v0.1.0.
