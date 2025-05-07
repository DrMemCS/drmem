# drmem-drv-latch

Provides devices that implement a latched value interface.

This driver is always available in DrMem.

## Configuration

This driver uses the following configuration parameters.

- `disabled` is the value of the `output` device when the latch is
  disabled. This value can be any type supported by DrMem devices.
- `enabled` is the value of the `output` device while the latch is
  active. This value can be any type supported by DrMem devices.

Both values must be of the same type.

## Devices

The driver creates these devices:

| Base Name | Type     | Units | Comment                                                                      |
|-----------|----------|-------|------------------------------------------------------------------------------|
| `trigger` | bool, RW |       | A `false` to `true` transition will set the `output` device to the `enabled` value. Changes to this device are ignored until the latch is reset. |
| `reset`   | bool, RW |       | When this input is `true`, the latch goes into the disabled state. |
| `output`  | T, RO    |       | Output state of timer.                                                       |

Every value sent to the `enable` device will be reported -- even
duplicates. This allows one to, if using the redis backend, see the
history of settings made to the device.

## History

Added in v0.5.0.
