# `drmem`

The Doctor Memory Project is an attempt to build a home automation
system -- a complete, easy-to-use control system for the home.

## Preparation

The core service used in this control system is
[Redis](https://redis.io/) which is light-weight and fast and has the
data management features needed for a control system: device
information, time-series storage of device readings, and routing
setting requests.

Before setting up `drmem`, you'll need to have a running instance of
`redis`. I set up my instance on my RaspberryPi to only listen on
127.0.0.1. If my control system grows to other RPis, `redis` can be
configured to listen on the local network. `redis` is an in-memory
database, so I've configured mine to periodically dump the database on
a NAS over NFS.

## Building

`drmem` is written in Rust using the excellent
[`tokio`](https://tokio.rs/) async scheduler module. To build it,
you'll need a Rust installation.

*NOTE: This project is in a very early state. Eventually I want users
to specify which drivers to use in the `drmem.conf` file. Right now
everything is getting built and run.*

## Colophon

The name of this project, `drmem`, is a shortened version of "Doctor
Memory" -- a character from Firesign Theatre's comedy album, "We're
All Bozos on this Bus". Doctor Memory is the marginally intelligent,
easily confused AI that operates behind the scenes of the Future Fair.

Although this project strives to be much more reliable than this
fictional character, I wanted to pay homage to Firesign Theatre's
early vision of a powerful control system.

... this is Worker speaking. Hello.
