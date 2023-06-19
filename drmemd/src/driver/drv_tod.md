# drmem-drv-tod

Provides devices that indicate the time-of-day. The driver can be
configured to generate UTC values or values based on the local
timezone.

This driver is always available in DrMem.

## Configuration

This driver uses the following configuration parameters.

- `utc` is a boolean which, when `true`, means the devices will
  represent UTC time. If `false`, the devices will represent local
  time.

## Devices

The driver creates these devices:

| Base Name     | Type    | Units | Comment                         |
|---------------|---------|-------|---------------------------------|
| `year`        | int, R0 |       | The year portion of the date.   |
| `month`       | int, R0 |       | The month portion of the date, ranging from 0 to 11. |
| `day`         | int, R0 |       | The day of the month portion of the date, ranging from 0 to 30. |
| `day-of-week` | int, R0 |       | The day of the week portion of the date, ranges from 0 to 6. 0 is Monday and 6 is Sunday. |
| `hour`        | int, R0 |       | The hour portion of the date. Ranges from 0 to 23. |
| `minute`      | int, R0 |       | The minute portion of the date. |
| `second`      | int, R0 |       | The second portion of the date. |

## History

Added in v0.3.1.
