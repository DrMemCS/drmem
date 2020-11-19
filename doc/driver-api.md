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

## API

Driver modules will provide a factory function to create a new driver
instance. This function will take driver-specific parameters which
provide addressing information in accessing its hardware. If the
function succeeds, it'll return a type which supports `dyn Driver`.

### The `Driver` trait

All driver instances will support the `Driver` trait which has these
methods:

`pub fn name(&self) -> &str`
: Returns a short name for the driver. The framework will append the
instance number.

`pub async fn report(&self) -> String`
: Generates a report, in Markdown format, of the running instance.
Driver authors should try to make the reports concise yet informative,
and should execute quickly. Report generation shouldn't hurt the
real-time response of the system (on multi-core systems, this isn't as
much an issue.)

This function is marked `async` because it may need access to shared
resources being used by the driver's main loop.

`pub async fn run(&mut self, ctxt : DbContext) -> Result<()>`
: This is called by the framework to activate the driver. It isn't
expected to return. If it does, it should use the `Error` enumeration
to report why it exited.

### `DbContext`

This manages the connection to the database. It has the following
methods:

`pub async fn get_device(&mut self, name : &str) -> Result<Device>`

`pub async fn get_periodic_events(&mut self, period : u64) -> Interval`

`pub async fn get_setting_events(&mut self, device : Device) -> Setting`

`pub async fn update(&mut self, stamp : u64, devs : &[Device]) -> Result<()>`

### `Device`

`type UpdateContext = (String, String)`

`fn toUpdateContext<T>(&self, val : T) -> UpdateContext`

### `device::Data`

```
pub enum Data {
    Bool(bool),                // <<$T>> | <<$F>>
    Int(i64),                  // <<$I, Val:64/signed-big>>
    Flt(f64),                  // <<$F, Val:64/float-big>>
    Str(String),               // <<$S, Size:24/unsigned-big, Val/binary>>
    BoolArr(Vec<bool>),        // <<$A, BOOL ...>>
    IntArr(Vec<i64>),          // <<$A, INT ...>>
    FltArr(Vec<f64>),          // <<$A, FLOAT ...>>
    StringArr(Vec<String>)     // <<$A, STRING ...>>
}

impl Data {
    fn from_string(s : &str) -> Data;

    fn to_string(&self) -> String;
}

trait ToData {
    pub fn to_data(&Self) -> Data;
}

trait FromData {
    pub fn from_data(data : Data) -> Option<Self>;
}
```
