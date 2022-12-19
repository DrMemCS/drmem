# Introduction

Hello and welcome to DrMem!

DrMem is a control system that uses minimal resources. In simplest
terms, "control system" is a system that intelligently controls
hardware based on the state of its inputs. Your thermostat, furnace,
and air conditioner for instance, form a tiny control system where
room temperature is used to decide when to turn the furnance, or air
conditioner, on and off.

For DrMem, however, we're using the more general term of "control
system" used by particle accelerators, observatories, power plants,
etc. In this larger definition, hardware support consists of "drivers"
that create a "device" which implements a standard API that can read
or write to the underlying hardware. At the lowest level, these
control systems have a large set of devices that can be accessed in
similar ways. Higher levels of the control system can archive device
information for later analysis. Applications can access live data or
read archived data. Devices also have reading limits so if they exceed
any limit, a control system typically has a way to report these
"alarms".

DrMem wants to meet these goals, but on a much smaller scale. It was
the author's desire to have the flexibility and reliability of a
control system, but be able to run it on a Raspberry Pi.

It should be noted that there is a difference between home automation
and a control system: a control system can do home automation tasks
but home automation systems can't perform all control system tasks.

DrMem can be configured to have a GraphQL interface so client
applications can read or set devices. This allows users to create
dashboards or applications that interact with DrMem.

DrMem also supports internal, "logic nodes" which are defined in the
configuration file. These nodes are lightweight data types that run in
the DrMem executable and which take device(s) as input, perform some
calculation, and write the results to another device. This allows a
DrMem installation to set up local control when it starts up.

## Other Projects

If you want to run control system software used by actual particle
accelerators, go to the EPICS home page. EPICS has been around since
the 90s and is used by laboratories around the world. DrMem took some
inspiration from this project, but tries to implement it in a much
simpler fashion.

If you are interested in home automation, there are many other
projects available as well.
