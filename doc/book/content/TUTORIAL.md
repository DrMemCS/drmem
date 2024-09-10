# Tutorial

For this tutorial, we'll build a very simple instance of `drmemd`. In
this version, we'll use the simple backend, no external drivers, and
include the GraphQL interface. We'll also enable the built-in GraphQL
editing interface.

This tutorial is based on v0.5.0.

## Build `drmemd`

Download and build the executable:

```
$ git clone git@github.com:DrMemCS/drmem.git
$ cd drmem
$ cargo build --features simple-backend,graphql
```

This builds the debug version which is found at `target/debug/drmemd`.

## Configure

`drmemd` looks for configuration information in a `drmem.toml`
file. This file can be in the current directory, your home directory
(as `.drmem.toml`), or in a system-wide location. For this tutorial,
we'll simply store it in the current directory.

Several small, useful drivers are always available so we'll use one of
them. Create the file `drmem.toml` with the following contents:

```
# These can be filled in later.

latitude = 0
longitude = 0

[graphql]
name = "tutorial"

[[driver]]
name = "timer"
prefix = "demo-timer"
cfg = { millis = 5000, disabled = false, enabled = true }
```

In the "driver" section, the "name" parameter specifies which driver
we're using for this instance. In this case, we're using the `timer`
driver which implememts a one-shot timer. The timer driver creates two
devices: `enable` and `output`.

The "prefix" parameter specifies the path to be prepended to the
device names created by this driver. For this configuration, the two
devices will be named `demo-timer:enable` and `demo-timer:output`.
Multiple "driver" sections can use the `timer` driver to make many
timer devices, but different prefixes need to be specified so all the
created devices have unique names.

Each driver has a "cfg" parameter which contains configuration
information specific to the driver. Each drivers' documentation will
specify what parameters are needed in the configuration. For timers,
we specify the length of its active time using the "millis" parameter
(in this example, 5 seconds.) The "enabled" parameter specifies the
value of the `output` device while the timing is active and "disabled"
specifies the value of the `output` when it's inactive.

## Run `drmemd`

Run the `drmemd` executable. It'll display some log messages as it
starts up. It also opens port 3000 for GraphQL clients.

## Interacting via GraphQL

When the `graphql` feature is used, `drmemd` provides a GraphQL API so
you can use GraphQL clients to interact with DrMem. There are free
GraphQL clients available, which provide a low-level, GraphQL
interface. Ambitious users can write web or mobile apps with GraphQL
client libraries to control and browse `drmemd` devices.

For this tutorial, download a free, GraphQL client for your desktop
platform. There are GraphQL extensions for the major browsers, as
well, so you could use one of them for this tutorial. (As of this
writing, the author has had success with the Altair extension for the
Chrome browser.)

Open your GraphQL client and go to "http://MACHINE:3000/drmem/q" --
replacing MACHINE with the name/address of the machine running
DrMem. If you can't connect, you may have to open port 3000 in your
firewall. If it successfully connects, you'll see an environment where
you can submit GraphQL queries and see the results.

Typically, on the right, are two tabs, "Docs" and "Schema". Clicking
on the "Docs" tab will show the GraphQL schema documentation which
describes available queries, their arguments, and the form of the
responses. Take a moment to peruse the docs.

## Drive Information

The first query we'll use is one which shows the drivers which are
available in the running instance of DrMem. In the query editor,
enter:

```
query {
  driverInfo {
    name
    summary
  }
}
```

and then press the "play" button. The results pane should show a list
of available drivers, including their name and a summary of what the
driver does. The "docs" show that the `DriverInfo` reply also includes
a "description" field. If you add that to the query, you'll get the
associated description of the driver (in Markdown format, so it's not
easy to read in this environment.)

## Device Information

There's also a query which returns information about the devices you
defined in your configuration file. In the query editor, enter:

```
query {
  deviceInfo {
    deviceName
    units
    settable
    driver {
      name
    }
  }
}
```

The result should be a list of two devices, both from the `timer`
driver. The "deviceName" field shows the names. "units" is `null`
because boolean device typically don't have engineering units
associated with them. "settable" indicates whether you can change the
device. "driver" contains information about the driver that implements
the device.

The `deviceInfo` query takes two, optional arguments which filter the
results of the query: `pattern` reduces the returned set to only
devices whose name matches the pattern; `settable` only returns
devices whose "settable" field matches the value of this argument.

## Getting Device Readings

If client applications are interested in the changing values of a
device, they can use the `monitorDevice()` query. This query uses the
GraphQL subscription service which means the query returns a stream of
results until the client closes the connection. DrMem devices only
return data when their value changes so a query may seem "stuck" or
"hung" but as long as the connection is there, you can assume there
haven't been any updates to the device.

Let's monitor the output of `demo-timer:output`. In the query panel,
enter:

```
subscription {
  monitorDevice(device:"demo-timer:output") {
    device
    stamp
    boolValue
  }
}
```

Monitoring a device always returns the current value along with the
timestamp when it occurred. Then it waits for further updates. We can
see these changes in the next section.

## Setting a Device

For a timer device, when the `enable` device goes from `false` to
`true`, the timer starts timing.

Put the current window, which is monitoring the `output` device, aside
and open another window (or tab, depending on your GraphQL client.)
Connect to DrMem using the same URL as before. In this window, we're
going to set the timer's `enable` device to `true` so it begins timing.

Enter this query:

```
mutation {
  control {
    setDevice (name: "demo-timer:enable", value: { bool: true })
  }
}
```

When you execute this query, you'll see the timer output goes
immediately to true and, after 5 seconds, goes back to false. To start
the timer again, you first have to set the `enable` device to `false`
before setting it to `true`.

GraphQL allows you to chain queries, but you have to add "alias"
labels so results can be matched with the query. Try this:

```
mutation {
  control {
    f:
    setDevice (name:"demo-timer:enable", value:{ bool:false })
    t:
    setDevice (name:"demo-timer:enable", value:{ bool:true })
  }
}
```

Now each time you run this double query, the timer resets; the first
query sets the `enable` device to `false` and the second to `true`.
The timer driver only reports *changes* to `output` so, if you reset
it before it times out, you'll see `output` change to false 5 seconds
later. In other words, the timer driver won't issue two `true` or two
`false` values.

## Summary

This tutorial shows how the GraphQL interface can be used to query
configuration information, receive updates to devices, and set devices
to new value (i.e. control devices.) All modern programming languages
have a GraphQL client library so you can use your favorite language to
interact with DrMem.

In [TUTORIAL2](TUTORIAL2.md), we'll explore a more advanced feature of
DrMem.
