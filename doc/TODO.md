# Framework

- [ ] Monitors tasks and restarts them if they fail.
- [ ] Provides driver registration and factory methods to create
      instances.
- [ ] Drivers are in shared libraries and loaded via configuration.

# Driver API

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

- [ ] Get device info.
- [ ] Get latest value.
- [ ] Get time range of data.
- [ ] Receive stream of updates.
- [ ] Set device info.
- [ ] Send settings.
- [ ] Needs to be in its own crate.

# Drivers

- [ ] Sump pump driver (really only interesting to me.)
  - [X] Needs to monitor sump pump and write results to redis.
  - [ ] Needs to use the final driver API
- [ ] Memory driver (allows typed locations to save settings and report
      as readings.)
- [ ] Weather driver
- [ ] System driver (monitors local machine's resources)
- [ ] Philips Hue driver.
- [ ] Tuya driver.
- [ ] NTPD driver.
- [ ] Move drivers into their own crate (once API crate is ready.)

# Reactive Engine

- [ ] Must apply device updates in timestamp order.
- [ ] Devices with the same timestamp must all be applied before
      updating logic.
