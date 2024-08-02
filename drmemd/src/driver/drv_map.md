# drmem-drv-map

A map device is a driver that maps a range of integers to a device
value. The driver creates two devices, `index` and `output`. When the
`index` is set, the driver finds the associated value and write both
of them to the backend storage. Every setting will get sent to the
backend -- even if the new value is the same as the old.

## Configuration

The configuration parameters for this driver are:

- `initial` is an optional parameter. It is an integer which specifies
  the initial index to use when the driver starts. If no initial value
  is specified, the `output` device will be set to the default value.
- `values` is an array of maps. Each map has three key/values: 1)
  'start' is an integer specifying the start of the index range, `end`
  is an optional parameter specifying the ending range, inclusive (if
  omitted, `end` is the same as `start`), and `value` is a value to
  output when the index device is set within this range.
- `default` is the value that will be used if the `index` device gets
  set to a value that doesn't lie within any range.

NOTE: when the driver starts, it makes sure that no range in the
`values` array overlaps any other range. It also makes sure that all
the values in the array and the default value are of the same type.

## Devices

The driver creates these devices:

| Base Name | Type       | Units | Comment                                                      |
|-----------|------------|-------|--------------------------------------------------------------|
| index  | integer, RW |       | The index value used to look up a value in the map. |
| output | VALUE, RO   |       | This device gets updated everytime the `index` device gets updated. It gets set to the value determined by the map lookup. If the index didn't lie in any range, it gets set to the default value. |

## History

Added in v0.4.0.
