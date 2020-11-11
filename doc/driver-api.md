# Driver API

Drivers in `drmem` are implemented as Rust `async` functions and will
be spawned as background tasks in `tokio`. In each driver task, there
will be some resource set-up followed by a main loop. The main loop
will acquire data from hardware and then write it to the store.

Each device has a "natural" event rate (i.e. a rate that makes the
most sense for the device.) Each iteration of the main loop in the
driver happens when the event fires. Right now, there are two event
types: periodic events that can't exceed 20 Hz and events which occur
when a setting is received. A setting event returns incoming settings
and the action is to write the setting value to the hardware (and then
also write it to the database.)

So the basic structure of a driver is:

- Initialize hardware resources
- Create event generator(s)
- For each event:
    - If event is periodic:
        - Get newest value(s) from hardware
        - Write value(s) to database
    - Else if event is a setting:
        - Set the appropriate hardware
        - Write value to database

The API should provide event combinators so that, for instance, a
periodic timer and a setting channel can be both monitored.

A driver writes to the database through an interface. The interface
enforces how keys are generated in `redis` and how the data is saved
in `redis`. This interface is also where the newest reading is
compared to the alarm limits and reported as in alarm.
