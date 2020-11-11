# Device Names

The control points in the system are called devices. Devices provide
values obtained by sensors or from data obtained over the network.
Some devices accept values which, in turn, control a piece of
hardware. In order to refer to a particular device in the control
system, they are given easy-to-remember names.

## Format of Name

Device names are a list of segments separated by colons. Segments are
made of alphanumeric and dash characters. These segments form the base
name of the device. A final segment, following a period, is a field
name.

    alphanum = ('a' .. 'z') | ('A' .. 'Z') | ('0' .. '9')
    
    segment = alphanum ('-' | alphanum)*
    
    device = segment (':' segment)* ('.' segment)?

### Questions

* Should we support renaming devices?

*Yes*

The `RENAMEX` command allows us to rename keys in `redis`. If we can
communicate with a running driver to change its devices' names, it
could use this command to rename while keeping historical data. But
the config file needs to be updated, too, otherwise the old key(s)
will be recreated at the next restart. It would be nice if both
details could be handled simultaneously.

* Should we support device name aliases?

*No*

With `redis`, aliasing would add extra round-trips to retrieve actual
data. Plus, this is supposed to be a simple control system. Just name
your devices correctly the first time.

## Fields

Field names should be short but can be anything. Some field names are
already defined and have specific meanings. Device driver authors can
create other field names, but they may not be well known by tools and
applications, so it is encouraged to use the well-known names.

The following field names are defined:

    `.summary`      one-line description of the device
    `.details`      detailed description of the device, in markdown
    `.location`     location of the device
    `.units`        engineering units for the device's reading
    `.value`        returns latest reading or setting from device

These names are virtual in that the full name with field isn't
necessarily stored anywhere. The APIs that access devices know how to
process the field names to access the underlying information (as
explained in the "Implementation Details" section.)

## Readings and Settings

A device is either read-only or read-write. The `.value` field is used
to interact with the device's underlying driver so it is special
(compared to other fields) in several ways:

* It is the default field if no field is specified. For instance,
  asking for `house:temperature` is the same as asking for
  `house:temperature.value`.

* It is the only field that is logged and so it has historical data
  that can be accessed.

* Reading this field will return the latest value in its
  history. Writing to this field forwards the value to the driver to
  control the hardware and gets written to the history.

Writing to a read-only device has no effect. The only way to tell is
that the history won't include the setting.

## Questions

* What is the data type of a device?

The code uses a Rust enumerated type to represent the various data
types. This type defines methods that encode to / decode from
strings so they can be stored in `redis`.

# Tools

A command line tool should be developed to interact with the device
database. It should allow devices to be added, modified and deleted.
It could also scan the database to find problems (i.e. a database
"linter".)

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

*Yes*

The driver framework will make sure the proper entries are made in
`redis`. If a key already exists, the framework will assume it's
already been properly created and won't change anything.

# Implementation Details

This control system is using REDIS for its backing store. Information
for devices will span keys since we can use different features of
REDIS to hold pieces of information.

Keys will start with the device's base name but not include the field
portion. Instead there will be one of a small set of names appended to
the key.

## Reading

Requests for the `.value` field will get routed to keys with `#hist`
appended. These keys hold time-series data for the device (i.e. a
redis stream type.) Asking for the `.value` field will result in the
most recent value to be pulled from the `#hist` key.

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

## Setting

Settings are handled by `redis` lists. If a driver indicates a
particular device accepts settings, the framework does the following:

- Creates an empty list with the key `NAME#setting`
- Creates an event generator that uses `BLPOP` to return values pushed
  to the list.
- The client interface uses `RPUSHX` to append a value to the setting
  list. This command fails if the list doesn't exist so the client
  will get immedaite feedback that the device doesn't accept settings.

### Questions

* How can we return errors to clients for bad settings?

The client interface could look at the `#info` key to get meta
information: is the setting value the same type as the device? Is the
value in range (provided we have standardized fields indicating valid
ranges)?

## Alarms

* Two alarm limits (`.alert_high`, `.alert_low`)? Or four
  (`.warn_high`, `.warn_low`, `.alert_high`, and `.alert_low`)?

* Whenever a record is written to the history, all four (two?) limits
  are compared and saved with the reading. If the state of the alarm
  has changed, it is published to the `drmem-alarm` channel.

## Future Ideas

Create a "Rule Processor" which manages and executes "rules". A rule
is where a writable device is the target of an expression of readable
devices.  The rule manager will monitor the channels of readable
devices and, when a value changes, the expression is re-evaluated and
the result written to the target device.
