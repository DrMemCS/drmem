# drmem-drv-ntp

This driver monitors the state of an NTP server and updates devices
with the latest information. It only reports information when the NTP
server has "sync-ed" with another time server.

The NTP server needs to be configured to use UDP communications.
Servers using broadcasts or multicasts to stay in sync will not
generate any updates from this driver.

It should be noted that, when these devices update, their timestamps
are suspect. The author's RPi -- after a reboot -- takes hours before
the time is within 1 ms of the remote time server. During that
stabilization time, the timestamps hop around by tens of milliseconds.
If your system is configured to write to a remote redis server, your
timestamps will be more consistent.

## Configuration

The driver needs to know the address of the NTP server. The NTP server
should be configured to accept query requests on the interface this
driver will access.

- `addr` is a string containing the host name, or IP address, and port
  number of the machine that's running the NTP service (in
  **"hostname:#"** or **"\#.#.#.#:#"** format.) The port is almost
  always 123.

## Devices

The driver creates these devices:

| Base Name | Type       | Units | Comment                                                   |
|-----------|------------|-------|--------------------------------------------------------------|
| `state`   | bool, RO   |       | Set to `true` when the system is sync-ed with a time server. |
| `source`  | string, RO |       | Set to the address of the sync-ed server.                    |
| `offset`  | f64, RO    | ms    | The offset of the current system-s time with the server's.   |
| `delay`   | f64, RO    | ms    | The estimated in-flight delay between the systems.           |

## History

Added in v0.1.0.
