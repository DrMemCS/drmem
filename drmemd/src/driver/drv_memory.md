# drmem-drv-memory

A memory device is simply an area in memory to store a value. Memory
devices are settable. Once set, the driver writes the value to the
backend storage. Every setting will get sent to the backend -- even if
the new value is the same as the old.

## Configuration

Since memory devices are similar to variables in a programming
laguage, this driver doesn't impose a final, device name but, instead,
lets the user decide. The only parameter is:

- `name` is a string containing the base name of the memory device.

## Devices

The driver creates these devices:

| Base Name | Type       | Units | Comment                                                      |
|-----------|------------|-------|--------------------------------------------------------------|
| NAME      | string, RW |       | The base name will be the name specified in the config file. |

## History

Added in v0.1.0.
