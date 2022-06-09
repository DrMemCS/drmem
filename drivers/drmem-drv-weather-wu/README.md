# drmem-drv-weather-wu

Periodically obtains weather information from the Weather Underground
website. Weather Underground has a network of 250,000+ weather
stations run by volunteers. In many parts of the world, one should be
able to find a nearby weather station. This means, however, the
associated devices will only update when the system has a working
internet connection.

## Configuration

These are the configuration parameters for an instance of the driver.

- `station` is a string containing the station ID.
- `key` is your Weather Underground API key. If this parameter isn't
  provided, a general key is used.
- `interval` is the number of minutes between each update. If a
  personal key isn't specified, the interval can't be less than 10
  minutes. If this parameter isn't provided, 10 minutes is used.
- `units` can be either "metric" or "imperial" and determines how the
  device data is scaled (i.e. Celsius or Fahrenheit, etc.)

## Devices

This driver creates a large set of devices. Depending upon the
associated weather station's capabilities, not all devices will get
updated. If your selected station isn't updating a device you need,
then you should try another station ID; Weather Underground has a huge
set of participating users so your location should have several useful
stations.

Like all drivers, when this driver starts up, it registers all its
devices. Part of the registration includes the units for each
device. For this driver, however, the set of units are determined by a
configuration parameter. If you change the config parameter and are
using the REDIS back-end, the units of the devices won't get updated
at a new restart. The simple back-end doesn't have any persistent
storage so each restart uses the current configuration.

NOTE: The author has seen at least one station provide garbage values
to Weather Underground and they simply save and report it. This
driver, therefore, does some sanity checks before updating a device.
For instance, it won't update the humidity device if it reads below 0%
or higher than 100%. When a parameter is deemed invalid, the
associated device won't get updated and a warning is written to the
log.

| Base Name | Type | Units | Comment |
|-----------|------|-------|---------|
| `dewpoint` | f64, RO | °F or °C | Dewpoint temperature |
| `heat-index` | f64, RO | °F or °C | Heat index temperature |
| `humidity` | f64, RO | % | Relative humidity |
| `precip-rate` | f64, RO | in/hr or mm/hr | Rate of precipitation |
| `precip-total` | f64, RO | in or mm | Integration of precipitation. Gets reset when `precip-rate` remains at 0 for two polls |
| `pressure` | f64, RO | in:Hg or hPa | Barometric pressure |
| `solar-rad` | f64, RO | W/m² | Solar radiation measurement. |
| `state`   | bool, RO | | Set to `true` while the system is able to communicate with Weather Underground. |
| `temperature` | f64, RO | °F or °C | Temperature |
| `uv`        | f64, RO | | UV undex |
| `wind-chill` | f64, RO | °F or °C | Wind chill temperature |
| `wind-dir` | f64, RO | ° | Wind direction (0° - 360°) |
| `wind-gust` | f64, RO | mph or km/h | Max wind speed recently measured. |
| `wind-speed` | f64, RO | mph or km/h | Wind speed |

## History

Added in v0.1.0.
