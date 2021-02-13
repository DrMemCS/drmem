# `drmemd` core

## Runtime Framework

The runtime framework is what's provided by `drmemd`. It loads drivers
specified by the configuration and gives them the environment in which
to run. It also routes device settings to the appropriate drivers.

- [ ] Monitors tasks and restarts them if they fail.
- [ ] Provides driver registration and factory methods to create
      instances.
- [ ] Rather than have every driver linked into `drmemd`, they should
      be shared libraries that are dynamically loaded, based on
      whether the configuration says they'll be needed (this is
      related to the later item where each driver should be in its own
      crate.)

## Client Interface

`drmemd` contains a web server which provides a GraphQL interface for
clients to use. This interface gives clients these abilities:

- [ ] Get latest value of a device.
- [ ] Get time range of data for a device.
- [ ] Receive stream of updates from a device.
- [ ] Send settings to devices.
- [ ] Get device info.
- [ ] Set device info.

# Driver API

The driver API defines the functions and data types drivers need to
use to interact with `redis`. It hides the details of how we map the
driver's worldview onto `redis` data types and capabilities.

- [X] Writes hardware state to redis.
- [ ] Receives settings (applies setting to hardware and writes to
      redis.)
- [X] Drivers can be specified in config file.
  - [ ] Test parsing (can it handle missing fields?)
  - [ ] Test address specification.
  - [ ] Test redis client/password ACLs
- [X] Address information can be specified in config file.
- [ ] Support register/unregister events
- [ ] Needs to be in its own crate.

# Drivers

This is a partial list of drivers that could be written for this
project.

- [ ] Sump pump driver (really only interesting to me.)
  - [X] Needs to monitor sump pump and write results to redis.
  - [ ] Needs to use the final driver API
  - [ ] Uses RPi GPIO. Maybe a GPIO driver could be written and this
        driver could use it.
- [ ] Memory driver (allows typed locations to save settings and report
      as readings.)
- [ ] Weather driver
  - [ ] There will probably be multiple drivers based on the source of
        the weather data. The author's initial effort would be called
	`drmem-dev-weather-noaa` as it uses METAR data from NOAA.
- [ ] System driver (monitors local machine's resources)
- [ ] Philips Hue driver. Includes not only Philips bulbs, switches, and
      motion sensors but any ZigBee device that happens to work with it.
- [ ] Tuya driver. An API used by many WiFi-based builbs, plugs, etc.
- [ ] NTPD driver.
- [ ] Move drivers into their own crate (once API crate is ready.)

# Reactive Engine

The RE is a standalone process that uses the client API to read and
set devices. It will read a source file that describes how inputs
should control outputs.

- [ ] Must apply device updates in timestamp order.
- [ ] Devices with the same timestamp must all be applied before
      updating logic.
