# Framework

The framework is what's provided by `drmemd`. It loads drivers
specified by the configuration and gives them the environment in which
to run.

- [ ] Monitors tasks and restarts them if they fail.
- [ ] Provides driver registration and factory methods to create
      instances.
- [ ] Drivers are in shared libraries and loaded via configuration.
      (this is related to the later item where each driver should
      be in its own crate.)

# Driver API

The driver API defines the functions and data types drivers should use
to interact with `redis`. It hides the details of how we map the
driver's worldview into `redis` data types.

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

# Client API

The client API, like the driver API, defines functions and data types
used to interface with `drmemd`. The purpose of this API, however, is
for reading device data and sending settings to drivers. This is a
lower-level interface than the proposed GraphQL interface.

- [ ] Get device info.
- [ ] Get latest value.
- [ ] Get time range of data.
- [ ] Receive stream of updates.
- [ ] Set device info.
- [ ] Send settings.
- [ ] Needs to be in its own crate.
- [ ] Should this only be GraphQL? No lower-level interface?

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
