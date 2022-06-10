# `drmemd` core

## Runtime Framework

The runtime framework is what's provided by `drmemd`. It loads drivers
specified by the configuration and gives them the environment in which
to run. It also routes device settings to the appropriate drivers.

- [X] Monitors tasks and restarts them if they fail.
- [X] Provides driver registration and factory methods to create
      instances.
- [ ] Rather than have every driver linked into `drmemd`, they should
      be shared libraries that are dynamically loaded, based on
      whether the configuration says they'll be needed.

## Client Interface

`drmemd` contains a web server which provides a GraphQL/gRPC interface
for clients to use. This interface gives clients these abilities:

- [ ] Get latest value of a device.
- [ ] Get time range of data for a device.
- [ ] Receive stream of updates from a device.
- [ ] Send settings to devices.
- [X] Get device info.
- [ ] Set device info.
- [X] Gets loaded drivers and their information.
- [X] Gets devices managed by this node.
- Security considerations
  - Devices can be marked public/private? Public devices are
    accessible to the Reactive Engine and external clients. Private
    devices are only accessible to the Reactive Engine.
  - Client connections should be encrypted
  - Add ACLs? This means there should be accounts/passwords?
  - Maybe client port is only accessible via loopback interface; then
    `ssh` can do the authentication?

# Driver API

The driver API defines the functions and data types drivers need to
use to interact with `redis`. It hides the details of how we map the
driver's worldview onto `redis` data types and capabilities.

- [X] When starting up, a driver instance does the following for each
      device it manages:
  - [X] If the device exists, it verifies the entry is valid
        (required fields present, proper types, etc.)
  - [X] If it doesn't exist, it creates the device entry and
        inserts a default value into its history.
- [X] Writes hardware state to redis.
- [X] Receives settings.
- [X] Drivers can be specified in config file.
  - [X] Test parsing (can it handle missing fields?)
  - [ ] Test address specification.
- [X] Address information can be specified in config file.
- [X] Needs to be in its own crate.

# Drivers

This is a partial list of drivers that could be written for this
project.

- [X] Sump pump driver (really only interesting to me.)
  - [X] Needs to monitor sump pump and write results to redis.
  - [X] Needs to use the final driver API
- [ ] Memory driver (allows typed locations to save settings and report
      as readings.)
- [X] Weather driver
  - [X] There will probably be multiple drivers based on the source of
        the weather data. The author's initial effort would be called
	    `drmem-drv-weather-wu` as it uses Weather Underground.
- [ ] System driver (monitors local machine's resources)
- [ ] Philips Hue driver. Includes not only Philips bulbs, switches, and
      motion sensors but any ZigBee device that happens to work with it.
- [ ] Tuya driver. An API used by many WiFi-based builbs, plugs, etc.
- [X] NTPD driver.
