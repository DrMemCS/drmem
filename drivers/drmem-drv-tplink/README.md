# drmem-drv-tplink

This driver supports WiFi devices that use the TP-Link protocol.

This driver doesn't access a user's tplink account to find the
hardware; DrMem wants to control things locally -- even when the
Internet isn't available. To make this work reliably, you should
assign a fixed address for the device using its MAC address and the
configuration in your DHCP service (typically in your router.)

## Tested Devices

| Device Model | Vendor | Description   |
|--------------|--------|---------------|
| HS220        | Kasa   | Dimmer switch |

## Configuration

The driver needs to know where to access the device.

- `addr` is a string containing the host name, or IP address, and port
  number of the TP-Link device (in **"hostname:#"** or
  **"\#.#.#.#:#"** format.) The port is almost always 9999.

## Devices

The driver creates these devices:

| Base Name    | Type     | Units | Comment                                |
|--------------|----------|-------|----------------------------------------|
| `error`      | bool, RO |       | If true, there is an error communicating with the device. |
| `brightness` | f64 , RW | %     | Accepts 0 - 100 for percent brightness. If the value is out of range, it will be brought back in range. |

## History

Added in v0.2.0.
