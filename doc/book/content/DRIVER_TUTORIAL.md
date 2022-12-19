# Writing Device Drivers

- Determine set of devices to be created by an instance of the driver
  - Should the set of devices match a list of standardized devices for
    the target hardware?
  - What are the types of the devices?
  - Which devices are settable?
  - How much history do you want to save?

- Create async task that implements the `Driver` trait.

- The `create()` method is called once to initialize the driver.
  - Set up persistent resources for the instance
  - Register the devices that are controlled by the instance

- The `run()` method gets called and isn't expected to return.
  - Driver can do practically anything -- it's an `async` task
  - Typically it sets up a `loop {}` with a use of the `tokio::select`
    macro to wait for one of several future to complete.
