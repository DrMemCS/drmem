This driver monitors the state of a sump pump through a custom,
non-commercial interface and updates a set of devices based on its
behavior.

The sump pump state is obatined via TCP with a RaspberryPi that's
monitoring a GPIO pin for state changes of the sump pump. It sends a
12-byte packet whenever the state changes. The first 8 bytes holds a
millisecond timestamp in big-endian format. The following 4 bytes
holds the new state.

With these packets, the driver can use the timestamps to compute duty
cycles and incoming flows rates for the sump pit each time the pump
turns off. The `state`, `duty`, and `in-flow` parameters are updated
simultaneously and, hence will have the same timestamps.

# Configuration

The driver needs to know where to access the remote service. It also
needs to know how to scale the results. Two driver arguments are used
to specify this information:

- `addr` is a string containing the host name, or IP address, and port
  number of the machine that's actually monitoring the sump pump (in
  **"hostname:#"** or **"\#.#.#.#:#"** format.)
- `gpm` is an integer that repesents the gallons-per-minute capacity
  of the sump pump. The pump owner's manual will typically have a
  table indicating the flow rate based on the rise of the discharge
  pipe.

# Devices

The driver creates these devices:

| Base Name | Type     | Units | Comment                                                   |
|-----------|----------|-------|-----------------------------------------------------------|
| `service` | bool, RO |       | Set to `true` when communicating with the remote service. |
| `state`   | bool, RO |       | Set to `true` when the pump is running.                   |
| `duty`    | f64, RO  | %     | Indicates duty cycle of last cycle.                       |
| `in-flow` | f64, RO  | gpm   | Indicates the in-flow rate for the last cycle.            |
