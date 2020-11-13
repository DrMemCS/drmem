These are some examples of devices:

## METAR Weather Driver

    weather:temperature         RO      float           F
    weather:dewpoint            RO      float           F
    weather:wind-speed          RO      float           mph
    weather:wind-dir            RO      float           degrees
    weather:precip-1hr          RO      float           in-hr
    weather:precip-6hr          RO      float           in-hr
    weather:precip-24hr         RO      float           in-hr
    weather:cloud-cover         RO      float[]         %[]

## Philips Hue Bulb

    bulb:brightness             RW      float           %
    bulb:color                  RW      float[]         %[]

## Wifi Outlet

    outlet:state                RW      bool

## Sump Pump Driver

    sump:state                  RO      bool
    sump:in-flow                RO      float           gpm
    sump:duty                   RO      float           %

## System Stats

    system:temperature          RO      float           F
    system:fan-speed            RO      float           rpm
    
    system:free-memory          RO      float           MB
    system:load-1min            RO      float           %
    system:load-5min            RO      float           %
    system:load-15min           RO      float           %
    
    system:net-outbound         RO      float           bytes
    system:net-inbound          RO      float           bytes
    
    system:ntp-delay            RO      float           sec
    system:ntp-jitter           RO      float           sec
    system:ntp-offset           RO      float           sec
    system:ntp-source           RO      string
    system:ntp-state            RO      bool

## `drmem` Info

    drmem:devices               RO      string[]
    drmem:drivers               RO      string[]

## Home Costs

Utility costs for your home could be manually added (or automated, if
your utility service provides a digital way to access the bill.) This
makes it possible to plot utility usage vs. weather. Or compare
utility usage year by year.

    utility:electric:usage      RO      float           kW
    utility:electric:cost       RO      float           $
    utility:water:usage         RO      float           cu-ft
    utility:water:cost          RO      float           $

## Stocks

You could store other real-time data, not related to your home at all.

    AAPL:open                   RO      float           $
    AAPL:close                  RO      float           $
    AAPL:high                   RO      float           $
    AAPL:low                    RO      float           $
    AAPL:volume                 RO      float           shares
