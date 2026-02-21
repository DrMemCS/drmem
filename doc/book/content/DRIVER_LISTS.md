# Driver List

DrMem "drivers" are software modules that interact with resources
(i.e. hardware, web data, etc.) and make them available to DrMem as a
set of "devices". This page describes the list of drivers that have
been created for DrMem.

Each of these drivers has a README file in its source directory so, if
you want more information, read the associated documentation.

## Internal Drivers

These drivers are unconditionally built and added to `drmemd` so they
will always be available. These tend to be simple, building-block
types of drivers so they don't add too much bloat and they're very
useful to have.

| Name   | Description                                         |
|--------|-----------------------------------------------------|
| cycle  | Generates a periodic true/false value               |
| map    | Translates ranges of integers into values           |
| latch  | Switches to an alternate state when an event occurs |
| memory | Can be set to any supported value                   |
| timer  | Generates am active signal for a length of time     |

## External Drivers

| Name       | Vendor | Model | Description                            |
|------------|--------|-------|----------------------------------------|
| ntp        |        | ntpd  | Monitors NTP server status             |
| sump       |        |       | Monitors sump pump using custom HW     |
| tplink     | Kasa   | HS220 | WiFi connected dimmer switch           |
| weather-wu |        |       | Acquires data from Weather Underground |
