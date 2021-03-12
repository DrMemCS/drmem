# `drmem`

The Doctor Memory Project strives to be a complete, easy-to-use
control system for home automation. Like the Arduino and RaspberryPi
communities, this project is aimed at the hobbyist that likes to
tinker and build systems. Although commercial products will be
supported, nothing prevents you from incorporating and controlling
your own custom hardware.

`drmem` has been developed with the following design goals:

* **Reliability**. Excepting hardware failures, this control system
should provide 24/7 service in controlling and monitoring its devices.
Like any project of this type, careful design and extensive testing
will help prevent issues. However, `drmem` will also be written in the
Rust programming language which provides strong compile-time checks
which eliminate whole classes of bugs that occur in other languages.
This project is an experiment in writing mission-critical code in
Rust.

* **Efficiency**. Because we're using Rust, we have a systems
programming language which generates optimal code and reduces CPU
usage. Less CPU means reduced power consumption and less latency in
responding to hardware inputs. In this project, we're also using the
`tokio` async scheduler which means tasks will get distributed across
all cores of the system, further reducing latencies (or providing more
scalability.)

* **Simplicity**. `drmem` is targeted for small installations so we
want to minimize the number services that need to be managed. It turns
out that the types available in `redis` can support most of the
use-cases in a control system, so that's the core of `drmem`. Add in
the process that interacts with hardware and an optional task that can
tie devices together with functional rules and you get a small, yet
powerful, control system.

* **Accessibility**. Although `drmem` is capable running in the
background with no user interaction, it is useful to have an interface
that applications can use to provide dashboards, etc. for viewing and
controlling `drmem` devices. This is provided by a built-in HTTP
server hosting a GraphQL interface.

NOTE: This project provides a general purpose control system. Meeting
timing constraints depends on how fast your system is and how many
devices are being accessed/controlled. It is *your* responsibility to
determine whether you need another node to handle the extra load. If
you write your own driver, *you* need to verify it works and meets
your requirements.

*THIS SOFTWARE IS PROVIDED "AS IS" AND WITHOUT ANY EXPRESS OR IMPLIED
WARRANTIES, INCLUDING, WITHOUT LIMITATION, THE IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE.*

## Preparation

The core service used in this control system is
[Redis](https://redis.io/) which is light-weight, *fast* and has the
data management features needed for a control system: device
information and time-series storage of device readings.

`redis` servers support multiple databases. The default database is
number 0 and is what `drmem` uses. For testing, other database numbers
can be used which isolates possibly buggy, development code from the
"operational" database.

Before setting up `drmem`, you'll need to have a running instance of
`redis`. The author configured an instance on a RaspberryPi to only
listen on 127.0.0.1. If the control system grows beyond one node,
`redis` can be configured to listen on the local network. `redis` is
an in-memory database, so the author configured it to periodically
dump the database on a NAS over NFS.

## Building

`drmem` is written in Rust using the excellent
[`tokio`](https://tokio.rs/) async scheduler module. To build it,
you'll need a Rust installation.

**NOTE: This project is in a very, very early state. Eventually I want
users to specify which drivers to use in `drmem.conf`. Right now every
driver gets built and run.**

Check out the source, run `cargo build --release`, and relax; this
takes  a half hour to build on my RPi 3+. On server boxes, it'll build
much faster.

Developers can run `cargo build` to create the debug version (found in
`target/debug/drmemd`).

## Colophon

The name of this project, `drmem`, is a shortened version of "Doctor
Memory" -- a character from Firesign Theatre's comedy album, "We're
All Bozos on this Bus". Doctor Memory is the marginally intelligent,
easily confused AI that operates behind the scenes of the Future Fair.

Although this project strives to be much more reliable than this
fictional character, I wanted to pay homage to Firesign Theatre's
early vision of a powerful control system.

... this is Worker speaking. Hello.
