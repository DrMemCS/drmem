![DrMem logo](assets/logo/drmem-header-small.png)

_A small, capable control system for the hobbyist_

[![MIT licensed][mit-badge]][mit-url]
[![Crates.io][crates-badge]][crates-url]
[![Build Status][actions-badge]][actions-url]
[![Twitter Follow][twitter-badge]][twitter-url]

[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/tokio-rs/tokio/blob/master/LICENSE
[crates-badge]: https://img.shields.io/crates/v/drmemd.svg
[crates-url]: https://crates.io/crates/drmemd
[actions-badge]: https://github.com/DrMemCS/drmem/actions/workflows/ci.yml/badge.svg
[actions-url]: https://github.com/DrMemCS/drmem/actions/workflows/ci.yml
[twitter-badge]: https://img.shields.io/twitter/follow/DrMemCS?style=social
[twitter-url]: https://twitter.com/DrMemCS

---

The DrMem Project strives to be a complete, easy-to-use control system
for the hobbyist. Like the Arduino and RaspberryPi communities, this
project is aimed at people that like to tinker and build programmable
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
server hosting a GraphQL interface.

## What's Ready Now

Although DrMem is a young project, it's core is very usable. These are
currently supported features:

- Two types of back-end storage
  - A "simple" back-end which only saves the last value of a device
  - A back-end which writes device information to a Redis server
- A client API which uses GraphQL
- Drivers which implement devices
  - 3 built-in drivers
    - Memory devices
	- Cycling device
	- Timer device
  - 3 external drivers
    - Weather Underground data
	- NTP daemon status
	- (Author's) custom sump pump monitor

You can try some of this out by following the
[tutorial](doc/book/content/TUTORIAL.md).

## Other Control System / Home Automation Projects

If you're looking for more complete solutions that are available now,
here are a few options:

- [EPICS](https://epics-controls.org/) is a professional control system used
  by particle accelerators and observatories around the world.
- [Mister House](https://github.com/hollie/misterhouse) is a home
  automation system, written in Perl, that has been around for
  decades.
- [Home Assistant](https://www.home-assistant.io) is a nice-looking,
  well-polished, home automation system.
- Many devices can also be controlled by [Google's Home app](https://play.google.com/store/apps/details?id=com.google.android.apps.chromecast.app)
  or [Apple's HomeKit app](https://www.apple.com/ios/home/)

But be sure to check back occasionally as `drmem` matures!

## Colophon

The name of this project, DrMem, is a shortened version of "Doctor
Memory" -- a character from Firesign Theatre's comedy album, "We're
All Bozos on this Bus". Doctor Memory is the marginally intelligent,
easily confused AI that operates behind the scenes of the Future Fair.

Although this project strives to be much more reliable than this
fictional character, I wanted to pay homage to Firesign Theatre's
early vision of a powerful control system.

... this is Worker speaking. Hello.
