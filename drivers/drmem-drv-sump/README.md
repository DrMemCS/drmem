# drmem-drv-sump

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

## Configuration

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

## Devices

The driver creates these devices:

| Base Name | Type     | Units | Comment                                                   |
|-----------|----------|-------|-----------------------------------------------------------|
| `service` | bool, RO |       | Set to `true` when communicating with the remote service. |
| `state`   | bool, RO |       | Set to `true` when the pump is running.                   |
| `duty`    | f64, RO  | %     | Indicates duty cycle of last cycle.                       |
| `in-flow` | f64, RO  | gpm   | Indicates the in-flow rate for the last cycle.            |

## Caveats

The remote process polls the state of the pump at 20Hz so the
timestamps will have 50 ms accuracy. Unfortunately the current switch
seems to have a little slop in how quickly it turns on. Depending upon
how many 60 hz cycles it takes to activate the relay, it could add 30
ms -- or more -- of latency. The relay in the current switch probably
has some delays, too. Lastly, it has been observed that long cycle
times (> 5 minutes) can vary by 10 seconds or more! This is probably
due to, when the pit fills slowly, the float and the attached switch
having tremendous slop when activating.

The takeaway is the measurements of the on/off times are probably
accurate to around 100 ms. But the float adds a lot of inaccuracy to
the measurements.
