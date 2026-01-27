# drmem-drv-memory

A memory device is an area in memory to store a value and are
settable. This driver allows you to define several memory devices in
one instance of the driver. All the defined memory devices will have
the same path prepended to their names. If you want a different path,
you need to create another instance of the driver.

When a memory device is set, the driver writes the value to the
backend storage. Every setting will get sent to the backend -- even if
the new value is the same as the old.

When you specify config parameters for each memory device, the
`initial` parameter not only determines the initial value the device
will take, but also defines the device's type. Trying to set the
device to a different type will be rejected as an error.

## Configuration

Since memory devices are similar to variables in a programming
language, this driver doesn't impose final, device names but, instead,
lets the user decide. The configuration parameter for this driver is:

- `vars` is an array of maps. Each map has two, required entries:
  - `name` is a string containing the base name of the memory device
  - `initial` is the initial value of the device. The value of this
    device will be the last value to which it was set or, if there is
    no history for the device, this initial value will be used.

If you change the type of the memory device (i.e. you modify the
configuration parameters so its `initial` value is of a different
type) then the driver will ignore any previous value from the history
and will set the device to the specified, initial value.

## Devices

The driver creates these devices: 

| Base Name | Type       | Units | Comment                                                      |
|-----------|------------|-------|--------------------------------------------------------------|
| NAME      | ?, RW |       | The base name will be the name specified in the config file. The device type will match the type used in the `initial` parameter. |

## History

Added in v0.1.0.
