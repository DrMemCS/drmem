# drmem-drv-memory

A memory device is an area in memory to store a value and are
settable. When set, the driver writes the value to the backend
storage. Every setting will get sent to the backend -- even if the new
value is the same as the old.

### Caveat

Memory devices accept all types of values supported by DrMem. You must
take care to only set the memory device to its expected value (based
on how you plan to use it.) For instance, say the configuration is set
up to store a color value in a memory device and logic blocks forward
this value to several LED bulbs. If the memory device gets set to a
string, the logic blocks will get type errors as they try to forward
the string to the bulbs.

## Configuration

Since memory devices are similar to variables in a programming
language, this driver doesn't impose a final, device name but,
instead, lets the user decide. The only parameter is:

- `name` is a string containing the base name of the memory device.

## Devices

The driver creates these devices:

| Base Name | Type       | Units | Comment                                                      |
|-----------|------------|-------|--------------------------------------------------------------|
| NAME      | string, RW |       | The base name will be the name specified in the config file. |

## History

Added in v0.1.0.
