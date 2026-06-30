# Agent Instructions

## Stack
- tokio

## Architecture
- `drmem-api` -- contains types and APIs used by drivers and the main executable
- `drivers` -- contains several crates that that implement DrMem drivers and can optionally be linked into `drmemd`
  - `drivers/drmem-drv-hue` -- a driver for the Philips Hue products
  - `drivers/drmem-drv-ntp` -- a driver to monitor NTPD servers
  - `drivers/drmem-drv-sump` -- a driver to monitor a sump pump
  - `drivers/drmem-drv-tplink` -- a driver to control devices that use the TP-Link protocol
  - `drivers/drmem-drv-weather-wu` -- a driver to retrieve weather info from Weather Underground
- `drmemd` -- builds the main executable `drmemd`
  - `drmemd/src/backends` -- holds code for storage backends; supports a simple or a redis backend, selectable by `cargo` features
  - `drmemd/src/logic` -- holds code implementing the "logic block" processes
  - `drmemd/src/drivers` -- holds several, simple drivers that are always available in a DrMem instance
  - `drmemd/src/graphql` -- defines the optional GraphQL interface

## Rules
- Prefer minimal patches
- Prefer functional solutions over imperative
- Run `cargo check` after code changes
- Do not edit generated files directly
- Run `make check` before committing
- Use `jj` for version control

## Commands
- Build: `cargo build --features simple-backend,all-drivers,graphql`
- Check: `cargo check --features simple-backend,all-drivers,graphql`
- Test: `cargo test --features simple-backend,all-drivers,graphql`
