These are some examples of devices:

## METAR Weather Driver

    weather:temperature		RO	float		F
    weather:dewpoint		RO	float		F
    weather:wind-speed		RO	float		mph
    weather:wind-dir		RO	float		degrees
    weather:precip-1hr		RO	float		in-hr
    weather:precip-6hr		RO	float		in-hr
    weather:precip-24hr		RO	float		in-hr
    weather:cloud-cover		RO	float[]		%[]

## Philips Hue Bulb

    bulb:brightness		RW	float		%
    bulb:color			RW	float[]		%[]

## Wifi Outlet

    outlet:state		RW	boolean

## Sump Pump Driver

    sump:state			RO	boolean
    sump:in-flow		RO	float		gpm
    sump:duty			RO	float		%

## System Stats

    system:temperature		RO	float		F
    system:fan-speed		RO	float		rpm
    system:free-memory		RO	float		MB
    system:load-1min		RO	float		%
    system:load-5min		RO	float		%
    system:load-15min		RO	float		%
    system:net-outbound		RO	float		bytes
    system:net-inbound		RO	float		bytes

## Home Costs

    utility:electric:usage	RO	float		kW
    utility:electric:cost	RO	float		$
    utility:water:usage		RO	float		cu-ft
    utility:water:cost		RO	float		$

## Stocks

    AAPL:open			RO	float		$
    AAPL:close			RO	float		$
    AAPL:high			RO	float		$
    AAPL:low			RO	float		$
    AAPL:volume			RO	float		shares
