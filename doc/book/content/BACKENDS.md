# Storage Backends

When building DrMem, you specify one of two storage backends to
include. These backends have different strengths and weaknesses but
both provide a limited history of device values for clients to access
and use.

## Simple

The "simple" backend is the easiest to include as it is implemented
100% within `drmemd` and doesn't require any external resources. The
simple backend has no persistence so each time the DrMem instance is
started, devices won't have any "current value" until their first
update occurs.

The simple backend has these features:

- Extremely fast -- GraphQL clients and logic blocks tap into the
  update channels so when devices report a new value, all the clients
  immediately get notified.
- Only remembers the last value of each device.

## Redis

If you want to preserve a histiry of device values, DrMem supports
writing the values to a Redis server.

The Redis backend has these features:

- Retains a limited history of each devices' history. When you
  configure a an instance of a device, you can specify the size of the
  history.
- As a backend, it's not as fast as the simple backend because GraphQL
  clients and logic blocks get device updates through Redis
  transactions.

NOTE: Even though the Redis backend is slower than the simple backend,
don't write it off as unusable. The author's DrMem configuration uses
the Redis backend. When multiple devices are updated, the timestamps
are all within a millisecond of each other. Redis is very fast and the
benefits of having device history outweight the extra latency that
Redis adds.
