# drmem-drv-tplink

This driver supports WiFi devices that use the TP-Link protocol.

This driver doesn't access a user's TP-Link account to find the
hardware; DrMem wants to control things locally -- even when the
Internet isn't available. To make this work reliably, you should
assign a fixed address for the device using its MAC address and the
configuration in your DHCP service (typically in your router.)

## Tested Devices

This table contains products that are known to work with this driver.

| Device Model | Vendor | Description   |
|--------------|--------|---------------|
| HS103        | Kasa   | On/off outlet |
| HS220        | Kasa   | Dimmer switch. Note: it takes around 200ms for this module to respond to a command. In some instances, it took ~1s! So don't try controlling it rapidly. |

## Configuration

The driver needs to know where to access the device.

- `addr` is a string containing the host name, or IP address, and port
  number of the TP-Link device (in **"hostname:#"** or
  **"\#.#.#.#:#"** format.) The port is almost always 9999.
- `type` is a string that indicates what type of device to use. It can
  be "switch", "outlet", or "dimmer".

## Devices

The driver creates these devices for switch devices:

| Base Name    | Type     | Units | Comment                                |
|--------------|----------|-------|----------------------------------------|
| `error`      | bool, RO |       | If true, there is an error communicating with the device. |
| `state`      | bool, RW |       | `true` and `false` turn it on and off, respectively. |
| `indicator`  | bool, RW |       | `true` and `false` turn the LED indicator on and off, respectively. |

The driver creates these devices for dimmer devices:

| Base Name    | Type     | Units | Comment                                |
|--------------|----------|-------|----------------------------------------|
| `error`      | bool, RO |       | If true, there is an error communicating with the device. |
| `brightness` | f64, RW  | %     | Accepts 0 - 100 for percent brightness. If the value is out of range, it will be brought back in range. |
| `indicator`  | bool, RW |       | `true` and `false` turn the LED indicator on and off, respectively. |

## History

Added in v0.3.0.
