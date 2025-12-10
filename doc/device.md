**NOTE: This is a document from the earliest days of the project. It's
contents are mostly out-of-date but it contains a few items that are
still of interest. Once all of it has been obsoleted or rolled into
the actual documentation, this file will get deleted.**

# Device Names

The control points in the system are called devices. Devices provide
values obtained by sensors or from data obtained over the network.
Some devices accept values which, in turn, control a piece of hardware
or alter the behavior of a driver. In order to refer to a particular
device in the control system, they are given easy-to-remember names.

## Format of Name

Device names are a list of segments separated by colons. Segments are
made of alphanumeric and dash characters. These segments form the
name of the device.

    alphanum = ('a' .. 'z') | ('A' .. 'Z') | ('0' .. '9')

    segment = alphanum ('-' | alphanum)*

    device = segment (':' segment)*

### Questions

* Should we support renaming devices?

*Probably* -- The `RENAMEX` command allows us to rename keys in
`redis`. If we can communicate with a running driver to change its
devices' names, it could use this command to rename while keeping
historical data. In the simple backend, the device information only
needs to be moved to a different key. But in both cases, the config
file needs to be updated, too, otherwise the old key(s) will be
recreated at the next restart. It would be nice if both details could
be handled simultaneously.

* Should we support device name aliases?

*No*

With `redis`, aliasing would add extra round-trips to retrieve actual
data. Plus, this is supposed to be a simple control system. Just name
your devices correctly the first time.

## Readings and Settings

A device is either read-only or read-write.

## Questions

* What is the data type of a device?

The code uses a Rust enumerated type to represent the various data
types. This type defines methods that encode to / decode from strings
so they can be stored in `redis`. The simple backend can store these
enumeration values directly.

# Drivers

Drivers should, eventually, be dynamically loaded. A site will declare
which drivers should be loaded from a config file. Each device driver
will take instance parameters -- most will be specific to the driver,
but all drivers will take a string which specifies the base name of
the device it controls.

For instance, a person living near Chicago may use a weather driver
that gets conditions from METAR data for the KORD (O'Hare
International Airport) station. The config file would require the
weather driver. The base name of the device would probably be
`weather` and the one parameter would be `"KORD"`.  The driver would
periodically get the weather and save the results to several devices:
`weather:temperature`, `weather:humidity`, `weather:precipitation`,
`weather.dewpoint`, etc.  possibly among others.

If a location wanted to track several weather stations, they could
specify the base names as `weather:kord` for one station and
`weather:kdpa` for another, for instance. The drivers would append the
same, final portions of the device names so there would be some
consistency.

### Questions

* Should a driver be responsible for creating missing devices in the
  database?

*Yes* -- The driver framework will make sure the proper entries are
made in `redis` and the simple backend. If a key already exists, the
framework will assume it's already been properly created and won't
change anything.

# Implementation Details

## REDIS backend

This control system is using REDIS for its backing store. Information
for devices will span keys since we can use different features of
REDIS to hold pieces of information.

### Reading

Requests for device values will get routed to keys with `#hist`
appended. These keys hold time-series data for the device (i.e. a
redis stream type.)

All other field names will get routed to a key with `#info` appended.
This key returns a hash map of which the field names for the device
are keys.

REDIS doesn't have a type system for stored values; they're strings.
Since clients don't get direct access to REDIS, the format of values
can include type information. This system uses a binary encoding to
preserve the type information. Although more complex types can be
built, the system will start by only supporting the following types:

    booleans, integers, UTF-8 strings, floats and arrays of the
    previous types

### Questions

* How can we return errors to clients for bad settings?

The client interface could look at the `#info` key to get meta
information: is the setting value the same type as the device? Is the
value in range (provided we have standardized fields indicating valid
ranges)?
