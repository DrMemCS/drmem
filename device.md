### Device Names

Device names are a list of segments separated by colons. Segments are
alphanumeric and can have special characters except colon and period
characters. A segment following a period is a field name.

Field names should be short but can be anything. Some field names are
already defined and have specific meanings. Device authors can create other
field names, but they may not be well known by tools and applications, so
it is encouraged to use the well-known names.

The following field names are defined:

	'.descr'	description of the device
	'.loc'		location of the device
	'.unit'		engineering units for the device's reading
	'.value'	returns latest reading from device

These names are virtual in that the full name with field isn't necessarily
stored anywhere. The APIs that access devices know how to process the field
names to access the underlying information (as explained in the next
section.)

## Tools

A command line tool should be developed to interact with the device database. It should allow devices to be added, modified and deleted. It could also scan the database to find problems (i.e. a database "linter".)

## Implementation Details

This control system is using REDIS for its backing store. Information for
devices will span keys since we can use different features of REDIS to hold
pieces of information.

Keys will start with the device name but not include the field name
portion. Instead there will be one of a small set of names appended to the
key.

Requests for the `.value` field will get routed to keys with `.hist`
appended. These keys hold time-series data for the device (i.e. a redis
stream type.) Asking for the `.value` field will result in the most recent
value to be pulled from the `.hist` key.

All other field names will get routed to a key with `.info` appended. This
key returns a hash map of which the field names for the device are keys.
