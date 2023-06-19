# drmem-drv-timer

Provides devices that implement a timer interface. Anytime the
`enable` device transitions from `false` to `true`, the output stays
active for a configured amount of time.

This driver is always available in DrMem.

## Configuration

This driver uses the following configuration parameters.

- `active_level` is a boolean which defines the output value when the
  timer is active. Once the timer expires, the output will be set to
  the complement of this.
- `millis` is the number of milliseconds the timer will remain
  active. NOTE: Officially, DrMem uses 20 Hz (50 ms) as it's fastest
  response time. Although you could specify a shorter time, you might
  be disappointed in its accuracy (depending on your system hardware
  and its CPU load.)

## Devices

The driver creates these devices:

| Base Name | Type     | Units | Comment                                                        |
|-----------|----------|-------|----------------------------------------------------------------|
| `enable`  | bool, RW |       | A `false` to `true` transition will reset and start the timer. |
| `output`  | bool, RO |       | Output state of timer.                                         |

Every value sent to the `enable` device will be reported -- even
duplicates. This allows one to, if using the redis backend, see the
history of settings made to the device. The `output` device, however,
only reports state changes. So if a client were to start the timer and
start it again before it expires, the `output` would only report the
initial active and then the final inactive values.

## History

Added in v0.1.0.
