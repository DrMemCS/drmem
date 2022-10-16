![DrMem logo](assets/logo/drmem-header-small.png)

_A small, capable control system for the hobbyist_

---

The DrMem Project strives to be a complete, easy-to-use control system
for home automation. Like the Arduino and RaspberryPi communities,
this project is aimed at the hobbyist that likes to tinker and build
systems. Although commercial products will be supported, nothing
prevents you from incorporating and controlling your own custom
hardware.

DrMem has been developed with the following design goals:

* **Reliability**. Excepting hardware failures, this control system
should provide 24/7 service in controlling and monitoring its devices.
Like any project of this type, careful design and extensive testing
will help prevent issues. However, DrMem will also be written in the
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

* **Simplicity**. DrMem is targeted for small installations so we
want to minimize the number services that need to be managed. The `drmemd`
executable, along with a configuration file that defines your location's
set of devices, is all you need.

* **Accessibility**. Although DrMem is capable running in the
background with no user interaction, it is useful to have an interface
that applications can use to provide dashboards, etc. for viewing and
controlling DrMem devices. This is provided by a built-in HTTP
server hosting a gRPC interface.

NOTE: This project provides a general purpose control system. Meeting
timing constraints depends on how fast your system is and how many
devices are being accessed/controlled. It is *your* responsibility to
determine whether you need another node to handle the extra load. If
you write your own driver, *you* need to verify it works and meets
your requirements.

*THIS SOFTWARE IS PROVIDED "AS IS" AND WITHOUT ANY EXPRESS OR IMPLIED
WARRANTIES, INCLUDING, WITHOUT LIMITATION, THE IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE.*

## Other Control System / Home Automation Projects

DrMem is a young project. If you're looking for more complete
solutions that are available now, here's a few options:

- [EPICS](https://epics.anl.gov) is a professional control system used
  in particle accelerators and observatories around the world.
- [Mister House](https://github.com/hollie/misterhouse) is a home
  automation system, written in Perl, that has been around for
  decades.
- [Home Assistant](https://www.home-assistant.io) is a nice-looking,
  well-polished, home automation system.
- Many devices can also be controlled by [Google's Home app](https://play.google.com/store/apps/details?id=com.google.android.apps.chromecast.app)
  or [Apple's Home app](https://www.apple.com/ios/home/)

But be sure to check back occasionally as `drmem ` matures!

## Preparation

The core service used in this control system is
[Redis](https://redis.io/) which is light-weight, *fast* and has the
data management features needed for a control system: device
information and time-series storage of device readings.

`redis` servers support multiple databases. The default database is
number 0 and is what DrMem uses. For testing, other database numbers
can be used which isolates possibly buggy, development code from the
"operational" database.

Before setting up DrMem, you'll need to have a running instance of
`redis`. The author configured an instance on a RaspberryPi to only
listen on 127.0.0.1. If the control system grows beyond one node,
`redis` can be configured to listen on the local network. `redis` is
an in-memory database, so the author configured it to periodically
dump the database on a NAS over NFS.

## Building

DrMem is written in Rust using the excellent
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

The name of this project, DrMem, is a shortened version of "Doctor
Memory" -- a character from Firesign Theatre's comedy album, "We're
All Bozos on this Bus". Doctor Memory is the marginally intelligent,
easily confused AI that operates behind the scenes of the Future Fair.

Although this project strives to be much more reliable than this
fictional character, I wanted to pay homage to Firesign Theatre's
early vision of a powerful control system.

... this is Worker speaking. Hello.
