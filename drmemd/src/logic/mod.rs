use drmem_api::{client, device, driver, Result};
use futures::future::{join_all, pending};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::{
    sync::{broadcast, oneshot, Barrier},
    task::JoinHandle,
};
use tokio_stream::{wrappers::BroadcastStream, StreamExt, StreamMap};
use tracing::{debug, error, info, info_span, warn};
use tracing_futures::Instrument;

use super::config;

mod compile;
pub mod solar;
pub mod tod;

// These are some helpful type aliases.

// The logic node will contain an array of these types. As readings
// come in, they'll be saved in the array.

type Inputs = Option<device::Value>;

// This is a set of streams that returns readings from all input
// devices.

type InputStream = StreamMap<usize, device::DataStream<device::Reading>>;

// Manages settings to a device. It makes sure we don't send duplicate
// settings and it encapsulates the request/reply transaction.

pub struct Output {
    prev: Option<device::Value>,
    chan: driver::TxDeviceSetting,
}

impl Output {
    // Creates a new `Output`. It takes ownership of the provided
    // setting channel and starts with its setting history cleared.

    pub fn create(chan: driver::TxDeviceSetting) -> Self {
        Output { prev: None, chan }
    }

    // Attempts to set the associated device to a new value.

    pub async fn send(&mut self, value: device::Value) -> bool {
        // Only attempt the setting if it is different than the
        // previous setting we sent.

        if let Some(prev) = self.prev.as_ref() {
            if *prev == value {
                return true;
            }
        }

        // Create the reply channel.

        let (tx_rpy, rx_rpy) = oneshot::channel();

        // Send the setting to the driver.

        if let Ok(()) = self.chan.send((value.clone(), tx_rpy)).await {
            match rx_rpy.await {
                Ok(Ok(v)) => {
                    // If the driver adjusted our setting, add a
                    // warning to the log.

                    if v != value {
                        warn!(
                            "driver adjusted setting from {} to {}",
                            &value, &v
                        )
                    }
                    self.prev = Some(value);
                    return true;
                }
                Ok(Err(e)) => error!("driver rejected setting : {}", &e),
                Err(e) => error!("setting failed : {}", &e),
            }
        } else {
            error!("driver not accepting settings")
        }
        false
    }
}

pub struct Node {
    inputs: Vec<Inputs>,
    in_stream: InputStream,
    time_ch: Option<tod::TimeFilter>,
    solar_ch: Option<broadcast::Receiver<solar::Info>>,
    def_exprs: Vec<compile::Program>,
    exprs: Vec<(compile::Program, Output)>,
}

impl Node {
    // Iterate through the input device mapping. As we work through
    // the list, build three things:
    //
    // 1) An array of the variable and definition names.
    //
    // 2) A chained set of streams which provide the readings.
    //
    // 3) An array of `Programs` which store their results in their
    // respective variable location.

    async fn setup_inputs(
        c_req: &client::RequestChan,
        vars: &HashMap<String, device::Name>,
        defs: &HashMap<String, String>,
    ) -> Result<(Vec<String>, InputStream, Vec<compile::Program>)> {
        let mut inputs = Vec::with_capacity(vars.len() + defs.len());
        let mut def_exprs = Vec::with_capacity(defs.len());
        let mut in_stream = StreamMap::with_capacity(vars.len());

        // Iterate through the input variable definitions. This maps
        // an indentifier name with a local, shorter name. For each
        // device, we get a monitor stream and add it to the set.

        for (vv, dev) in vars {
            match c_req.monitor_device(dev.clone(), None, None).await {
                Ok(s) => {
                    // Use the total elements in `inputs` as the
                    // key. As elements are added to the vector, this
                    // value gets incremented. When pulling values
                    // from the StreamMap, the key is returned, which
                    // is also the index in the vector, so we know
                    // which entry to update.

                    in_stream.insert(inputs.len(), s);
                    inputs.push(vv.clone());

                    debug!("inp[{}] = {}", inputs.len(), &dev)
                }
                Err(e) => {
                    error!("error mapping '{}' to '{}': {}", &vv, &dev, &e);
                    return Err(e);
                }
            }
        }

        // Now add the definitions to the vector of inputs (we've
        // already verified the 'defs' names don't conflict with
        // 'inputs' names.)

        for (name, expr) in defs {
            // Add the definition's target name to the list of names.

            inputs.push(name.clone());

            // Compile the expression. The length of the input slice
            // is clipped to the size of the input variables. We do
            // this so we don't include any variables created by
            // definitions. This includes loops (a definition
            // referring to itself) and referring to other defintions
            // (because we can't enforce an order of evaluation.) The
            // "outputs" are also the inputs since `defs` calculate
            // values used by expressions and save their result in an
            // input parameter.

            let env = (&inputs[..vars.len()], &inputs[..]);
            let result = compile::Program::compile(
                &format!("{} -> {{{}}}", &expr, &name),
                &env,
            )
            .map(compile::Program::optimize)?;

            if result.0 == compile::Expr::Nothing {
                warn!("expression '{}' never generates a value", &expr);
            } else {
                debug!("inp[{}] = {}", result.1, &result.0);
            }

            // Add the program to the list of programs.

            def_exprs.push(result);
        }

        Ok((inputs, in_stream, def_exprs))
    }

    async fn setup_outputs(
        c_req: &client::RequestChan,
        vars: &HashMap<String, device::Name>,
    ) -> Result<(Vec<String>, Vec<Output>)> {
        let mut outputs = Vec::with_capacity(vars.len());
        let mut out_chans = Vec::with_capacity(vars.len());

        for (vv, dev) in vars {
            match c_req.get_setting_chan(dev.clone(), false).await {
                Ok(ch) => {
                    // Use the total elements in `inputs` as the
                    // key. As elements are added to the vector, this
                    // value gets incremented. When pulling values
                    // from the StreamMap, the key is returned, which
                    // is also the index in the vector, so we know
                    // which entry to update.

                    out_chans.push(Output::create(ch));
                    outputs.push(vv.clone());

                    debug!("out[{}] controls {}", outputs.len(), &dev)
                }
                Err(e) => {
                    error!("error mapping '{}' to '{}': {}", &vv, &dev, &e);
                    return Err(e);
                }
            }
        }
        Ok((outputs, out_chans))
    }

    // Creates an instance of `Node` and initializes its state using
    // the configuration information.

    async fn init(
        c_req: client::RequestChan,
        c_time: broadcast::Receiver<tod::Info>,
        c_solar: broadcast::Receiver<solar::Info>,
        cfg: config::Logic,
    ) -> Result<Node> {
        debug!("compiling expressions");

        if cfg.exprs.is_empty() {
            return Err(drmem_api::Error::ConfigError(
                "configuration doesn't define any expressions".into(),
            ));
        }

        // Validate the inputs.
        //
        // We add the names of the `inputs` and `defs` variables to a
        // set. If a name is already in the set, we return an
        // error. We add the devices in another set and make sure all
        // are unique.

        {
            use std::collections::HashSet;

            let mut name_set: HashSet<&String> =
                HashSet::with_capacity(cfg.inputs.len() + cfg.defs.len());
            let mut dev_set: HashSet<&device::Name> =
                HashSet::with_capacity(cfg.inputs.len());

            for (ref k, ref v) in &cfg.inputs {
                if !name_set.insert(k) {
                    return Err(drmem_api::Error::ConfigError(format!(
                        "name '{}' is defined more than once in 'inputs'",
                        k
                    )));
                }
                if !dev_set.insert(v) {
                    return Err(drmem_api::Error::ConfigError(format!(
                        "device '{}' is defined more than once in 'inputs'",
                        v
                    )));
                }
            }

            for ref k in cfg.defs.keys() {
                if !name_set.insert(k) {
                    return Err(drmem_api::Error::ConfigError(format!(
                        "'{}' is defined in 'defs' and 'inputs' sections",
                        k
                    )));
                }
            }
        }

        // Validate the outputs.
        //
        // We add the names of the `outputs` variables to a set. If a
        // name is already in the set, we return an error. We add the
        // devices in another set and make sure all are unique.

        {
            use std::collections::HashSet;

            if cfg.outputs.is_empty() {
                return Err(drmem_api::Error::ConfigError(
                    "configuration doesn't define any outputs".into(),
                ));
            }

            let mut name_set: HashSet<&str> =
                HashSet::with_capacity(cfg.outputs.len());
            let mut dev_set: HashSet<&device::Name> =
                HashSet::with_capacity(cfg.outputs.len());

            for (ref k, ref v) in &cfg.outputs {
                if !name_set.insert(k.as_str()) {
                    return Err(drmem_api::Error::ConfigError(format!(
                        "name '{}' is defined more than once in 'outputs'",
                        k
                    )));
                }
                if !dev_set.insert(v) {
                    return Err(drmem_api::Error::ConfigError(format!(
                        "device '{}' is defined more than once in 'outputs'",
                        v
                    )));
                }
            }
        }

        let (inputs, in_stream, def_exprs) =
            Node::setup_inputs(&c_req, &cfg.inputs, &cfg.defs).await?;

        let (outputs, out_chans) =
            Node::setup_outputs(&c_req, &cfg.outputs).await?;

        // Create the input/output environment that the compiler can
        // use to compute the variables in the expression.

        let env = (&inputs[..], &outputs[..]);

        // Iterate through the vector of strings. For each, compile it
        // into a `Program` type. Report the success or failure.

        let exprs: Result<Vec<compile::Program>> = cfg
            .exprs
            .iter()
            .map(|s| {
                compile::Program::compile(s.as_str(), &env)
                    .map(compile::Program::optimize)
            })
            .inspect(|e| match e {
                Ok(ex) => {
                    if ex.0 == compile::Expr::Nothing {
                        warn!(
                            "expression for out[{}] never generates a value",
                            ex.1
                        )
                    } else {
                        debug!("out[{}] = {}", ex.1, &ex.0)
                    }
                }
                Err(e) => error!("{}", &e),
            })
            .collect();
        let mut exprs = exprs?;

        // Sort the expressions based on the index of the outputs. The
        // output variables are in a hash map, so the vector is built
        // in whatever order the map uses. This might not be the same
        // order that the expressions are given. By sorting the
        // expressions, we line them up so they can be zipped together
        // later in this function.
        //
        // XXX: This should be refactored. The parser should return
        // the output variable name instead of an index in the output
        // environment. Then we should go through the expressions, in
        // order, and add the target output to the output vector. This
        // would have two benefits:
        //
        // 1) Even though multiple expressions send their results at
        // roughly the same time, the actual settings would go out
        // quickly in expression order.
        //
        // 2) If the user specified more output variables than
        // expressions that use them, resources wouldn't be allocated
        // for unused output devices.

        exprs[..].sort_unstable_by(
            |compile::Program(_, a), compile::Program(_, b)| a.cmp(b),
        );

        // Look at each expression and see if it needs the
        // time-of-day.

        let needs_time = exprs
            .iter()
            .chain(&def_exprs)
            .filter_map(|compile::Program(e, _)| e.uses_time())
            .min();

        // Look at each expression and see if it needs any solar
        // information.

        let needs_solar = exprs
            .iter()
            .chain(&def_exprs)
            .any(|compile::Program(e, _)| e.uses_solar());

        // Return the initialized `Node`.

        Ok(Node {
            inputs: vec![None; inputs.len()],
            in_stream,
            time_ch: needs_time
                .map(|tf| tod::time_filter(BroadcastStream::new(c_time), tf)),
            solar_ch: if needs_solar { Some(c_solar) } else { None },
            def_exprs,
            exprs: exprs.drain(..).zip(out_chans).collect(),
        })
    }

    // Runs the node logic. This method should never return.

    async fn run(mut self) -> Result<Infallible> {
        let mut time = Arc::new((chrono::Utc::now(), chrono::Local::now()));
        let mut solar = None;

        info!("starting");

        loop {
            // Create a future that yields the time-of-day using the
            // TimeFilter. If no expression uses time, then `time_ch`
            // will be `None` and we return a future that never
            // resolves.

            let wait_for_time = async {
                match self.time_ch.as_mut() {
                    None => pending().await,
                    Some(s) => s.next().await,
                }
            };

            // Create a future that yields the next solar update. If
            // no expression uses solar data, `solar_ch` will be
            // `None` and we, instead, return a future that never
            // resolves.

            let wait_for_solar = async {
                match self.solar_ch.as_mut() {
                    None => pending().await,
                    Some(ch) => ch.recv().await,
                }
            };

            #[rustfmt::skip]
	    tokio::select! {
		biased;

		// If we need the solar channel, wait for the next
		// update.

		v = wait_for_solar => {
		    match v {
			Ok(v) => solar = Some(v),
			Err(broadcast::error::RecvError::Lagged(_)) => {
			    warn!("not handling solar info fast enough");
			    continue
			}
			Err(broadcast::error::RecvError::Closed) => {
			    error!("solar info channel is closed");
			    return Err(drmem_api::Error::OperationError(
				"solar channel closed".into()
			    ));
			}
		    }
		}

		// If we need the time channel, wait for the next
		// second.

		Some(v) = wait_for_time => {
		    time = v;
		}

		// Wait for the next reading to arrive. All the
		// incoming streams have been combined into one and
		// the returned value is a pair consisting of an index
		// and the actual reading.

		Some((idx, reading)) = self.in_stream.next() => {
		    // Save the reading in our array for future
		    // recalculations.

		    self.inputs[idx] = Some(reading.value);
		}
	    }

            // Calculate each expression of the `defs` array. Store
            // each expression's result in the associated `input`
            // cell.

            self.def_exprs
                .iter()
                .for_each(|compile::Program(expr, idx)| {
                    self.inputs[*idx] =
                        compile::eval(expr, &self.inputs, &time, solar.as_ref())
                });

            // Calculate each of the final expressions. If there are
            // more than one expressions in this node, they are
            // evaluated concurrently.

            join_all(self.exprs.iter_mut().filter_map(
                |(compile::Program(expr, _), out)| {
                    compile::eval(expr, &self.inputs, &time, solar.as_ref())
                        .map(|v| out.send(v))
                },
            ))
            .await;
        }
    }

    // Starts a new instance of a logic node.

    pub fn start(
        c_req: client::RequestChan,
        rx_tod: broadcast::Receiver<tod::Info>,
        rx_solar: broadcast::Receiver<solar::Info>,
        cfg: config::Logic,
        barrier: Arc<Barrier>,
    ) -> JoinHandle<Result<Infallible>> {
        let name = cfg.name.clone();

        // Put the node in the background.

        tokio::spawn(
            async move {
                let name = cfg.name.clone();

                // Create a new instance and let it initialize itself.
                // Hold onto the result -- success or failure -- and
                // handle it after the barrier.

                let node = Node::init(c_req, rx_tod, rx_solar, cfg)
                    .instrument(info_span!("init", name = &name))
                    .await;

                // This barrier syncs this tasks with the start-up
                // task. When both wait on the barrier, they both wake
                // up and continue. The start-up task then knows this
                // logic block has registered all the devices, tod,
                // and solar handles it needs.

                barrier.wait().await;

                // Enter the main loop of the logic block.
                //
                // NOTE: We used the '?' operator here instead of the
                // assignment above because we have to wait on the
                // barrier. If we let the '?' operator return before
                // waiting on the barrier, the initialization loop
                // would wait forever.

                node?.run().await
            }
            .instrument(info_span!("logic", name)),
        )
    }
}

#[cfg(test)]
mod test {
    use super::{config, solar, tod, Node};
    use drmem_api::{
        client::{self, Request},
        device, driver, Error, Result,
    };
    use futures::Future;
    use std::{collections::HashMap, sync::Arc, time::Duration};
    use tokio::{
        sync::{broadcast, mpsc, oneshot, Barrier},
        task, time,
    };
    use tokio_stream::{wrappers::ReceiverStream, StreamExt};

    // This type implements an emulator of the DrMem core. It will
    // spin up a logic block using the provided configuration. The
    // unit tests can provide channels to send readings and receive
    // settings and verify correct operation.

    struct Emulator {
        inputs: HashMap<Arc<str>, mpsc::Receiver<device::Value>>,
        outputs: HashMap<Arc<str>, driver::TxDeviceSetting>,
    }

    impl Emulator {
        pub async fn start(
            inputs: Vec<(Arc<str>, mpsc::Receiver<device::Value>)>,
            outputs: Vec<(Arc<str>, driver::TxDeviceSetting)>,
            cfg: config::Logic,
        ) -> Result<(
            broadcast::Sender<tod::Info>,
            broadcast::Sender<solar::Info>,
            task::JoinHandle<Result<bool>>,
            oneshot::Sender<()>,
        )> {
            Emulator::new(inputs, outputs).launch(cfg).await
        }

        // Creates a new instance of an Emulator and loads it with the
        // input and output names and channels.

        fn new(
            mut inputs: Vec<(Arc<str>, mpsc::Receiver<device::Value>)>,
            mut outputs: Vec<(Arc<str>, driver::TxDeviceSetting)>,
        ) -> Self {
            Emulator {
                inputs: HashMap::from_iter(inputs.drain(..)),
                outputs: HashMap::from_iter(outputs.drain(..)),
            }
        }

        // Launches a logic block with the provided configuration.

        async fn launch(
            mut self,
            cfg: config::Logic,
        ) -> Result<(
            broadcast::Sender<tod::Info>,
            broadcast::Sender<solar::Info>,
            task::JoinHandle<Result<bool>>,
            oneshot::Sender<()>,
        )> {
            // Create the common channels used by DrMem.

            let (tx_req, mut c_recv) = mpsc::channel(100);
            let (tx_tod, _) = broadcast::channel(100);
            let (tx_solar, _) = broadcast::channel(100);

            let barrier = Arc::new(Barrier::new(1));

            // Start the logic block with the proper communciation
            // channels and configuration.

            let node = Node::start(
                client::RequestChan::new(tx_req),
                tx_tod.subscribe(),
                tx_solar.subscribe(),
                cfg,
                barrier,
            );

            // Create the 'stop' channel.

            let (tx_stop, rx_stop) = oneshot::channel();

            let emu = task::spawn(async move {
                // The first responsibility is to handle the
                // initialization of the node. This loop iterates
                // through and satisfies the requests made by the
                // node.

                loop {
                    // Did we get a request message? Handle it.

                    if let Some(message) = c_recv.recv().await {
                        match message {
                            Request::GetSettingChan {
                                name, rpy_chan, ..
                            } => {
                                let name = name.to_string();
                                let _ = rpy_chan.send(
                                    if let Some(tx) =
                                        self.outputs.remove(name.as_str())
                                    {
                                        Ok(tx)
                                    } else {
                                        Err(Error::NotFound)
                                    },
                                );
                            }
                            Request::QueryDeviceInfo { rpy_chan, .. } => {
                                let _ = rpy_chan.send(Err(
                                    Error::ProtocolError("bad request".into()),
                                ));
                            }
                            Request::SetDevice { rpy_chan, .. } => {
                                let _ = rpy_chan.send(Err(
                                    Error::ProtocolError("bad request".into()),
                                ));
                            }
                            Request::MonitorDevice {
                                name, rpy_chan, ..
                            } => {
                                let name = name.to_string();
                                let _ = rpy_chan.send(
                                    if let Some(rx) =
					self.inputs.remove(name.as_str())
                                    {
					let stream =
                                            Box::pin(ReceiverStream::new(rx).map(
						|v| device::Reading {
                                                    ts: std::time::SystemTime::now(
                                                    ),
                                                    value: v,
						},
                                            ));

					Ok(stream as device::DataStream<device::Reading,>)
                                    } else {
					Err(Error::NotFound)
                                    }
				);
                            }
                        }
                    }
                    // If the channel returned `None`, then the node
                    // dropped the channel sender (indicating its
                    // initialization is done.) If both hash maps
                    // aren't empty, then the node didn't ask for all
                    // the resources we provided (could be the
                    // configuration was incorrect.) In any case,
                    // return `false`.
                    else if !self.outputs.is_empty()
                        || !self.inputs.is_empty()
                    {
                        return Ok(false);
                    }
                    // All is good. Break out of the loop.
                    else {
                        break;
                    }
                }

                let ah = node.abort_handle();

                #[rustfmt::skip]
                tokio::select! {
		    // Look for the signal from the unit test to tell
		    // us to exit.

                    _ = rx_stop => (),

		    // If the Node exits, it's due to an error. Report
		    // the error.

                    v = node =>
			return match v {
                            Ok(result) => result.map(|_| false),
                            Err(e) => Err(Error::OperationError(
				format!("logic block panicked: {}", &e)
			    ))
			}
                }

                ah.abort();
                Ok(true)
            });

            Ok((tx_tod, tx_solar, emu, tx_stop))
        }
    }

    // Test sending a setting. The settings pass through an Output
    // channel and get debounced.

    #[tokio::test]
    async fn test_send() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut o = super::Output::create(tx);
        let h = task::spawn(async move {
            assert_eq!(o.send(device::Value::Bool(true)).await, true);
            assert_eq!(o.send(device::Value::Bool(true)).await, true);
            assert_eq!(o.send(device::Value::Bool(false)).await, true);
        });

        let (v, tx) = rx.recv().await.unwrap();

        assert_eq!(v, device::Value::Bool(true));
        assert!(tx.send(Ok(v)).is_ok());

        let (v, tx) = rx.recv().await.unwrap();

        assert_eq!(v, device::Value::Bool(false));
        assert!(tx.send(Ok(v)).is_ok());

        h.await.unwrap();
    }

    // Builds a future that will create a node. When awaiting on this
    // future, another task needs to handle potential requests
    // initiated by the node, over the `mpsc_rx` channel. These
    // requests can be monitoring requests for other devices or
    // requests for a setting channel to a device.

    fn init_node<'a>(
        cfg: config::Logic,
    ) -> (
        impl Future<Output = Result<Node>> + 'a,
        mpsc::Receiver<client::Request>,
        broadcast::Sender<tod::Info>,
        broadcast::Sender<solar::Info>,
    ) {
        let (mpsc_tx, mpsc_rx) = mpsc::channel(10);
        let c_req = client::RequestChan::new(mpsc_tx);
        let (tod_tx, c_time) = broadcast::channel(10);
        let (sol_tx, c_solar) = broadcast::channel(10);
        let node_fut = Node::init(c_req, c_time, c_solar, cfg);

        (node_fut, mpsc_rx, tod_tx, sol_tx)
    }

    // Builds a `config::Logic` type using arrays of config
    // parameters. This is much cleaner than doing it inline and
    // trying to get the final collection types.

    fn build_config(
        inputs: &[(&str, &str)],
        outputs: &[(&str, &str)],
        defs: &[(&str, &str)],
        exprs: &[&str],
    ) -> config::Logic {
        config::Logic {
            name: "test".into(),
            summary: None,
            inputs: inputs
                .iter()
                .map(|&(a, b)| (a.into(), device::Name::create(b).unwrap()))
                .collect(),
            outputs: outputs
                .iter()
                .map(|&(a, b)| (a.into(), device::Name::create(b).unwrap()))
                .collect(),
            defs: defs.iter().map(|&(a, b)| (a.into(), b.into())).collect(),
            exprs: exprs.iter().map(|&a| a.into()).collect(),
        }
    }

    // This tests the initialization of an empty configuration. The
    // main tests are to see if the two broadcast receivers are
    // dropped.

    #[tokio::test]
    async fn test_bad_config() {
        // Test that we reject an empty configuration (mainly because
        // if there are no expressions, there is nothing to do.)

        {
            let cfg = build_config(&[], &[], &[], &[]);
            let (node, _, tod_tx, sol_tx) = init_node(cfg);

            tokio::pin!(node);

            // `await` on the future. This should return immediately
            // as an error because there are no expressions to
            // process.

            assert!(matches!(node.as_mut().await, Err(Error::ConfigError(_))));

            // With no config, the TOD and solar channel handles
            // should have been dropped.

            assert_eq!(tod_tx.receiver_count(), 0);
            assert_eq!(sol_tx.receiver_count(), 0);

            // This call allows us to keep ownership of the node so
            // we're sure the Node dropped the broadcast receivers and
            // not from an early drop of `node` itself.

            std::mem::drop(node);
        }

        // Test that we reject two inputs with the same device.

        {
            let cfg = build_config(
                &[("in", "device:in"), ("in2", "device:in")],
                &[("out", "device:out")],
                &[],
                &["{in} -> {out}"],
            );
            let (node, _, _, _) = init_node(cfg);

            // `await` on the future. This should return immediately
            // as an error because there are no expressions to
            // process.

            assert!(matches!(node.await, Err(Error::ConfigError(_))));
        }

        // Test that we reject an input and a def with the same name.

        {
            let cfg = build_config(
                &[("in", "device:in")],
                &[("out", "device:out")],
                &[("in", "{in}")],
                &["{in} -> {out}"],
            );
            let (node, _, _, _) = init_node(cfg);

            // `await` on the future. This should return immediately
            // as an error because there are no expressions to
            // process.

            assert!(matches!(node.await, Err(Error::ConfigError(_))));
        }

        // Test that we reject two outputs with the same device.

        {
            let cfg = build_config(
                &[("in", "device:in")],
                &[("out", "device:out"), ("out2", "device:out")],
                &[],
                &["{in} -> {out}"],
            );
            let (node, _, _, _) = init_node(cfg);

            // `await` on the future. This should return immediately
            // as an error because there are no expressions to
            // process.

            assert!(matches!(node.await, Err(Error::ConfigError(_))));
        }
    }

    // Test a basic logic block in which an input device's value is
    // forwarded to an output device.

    #[tokio::test]
    async fn test_basic_node() {
        let cfg = build_config(
            &[("in", "device:in")],
            &[("out", "device:out")],
            &[],
            &["{in} -> {out}"],
        );
        let (tx_in, rx_in) = mpsc::channel(100);
        let (tx_out, mut rx_out) = mpsc::channel(100);

        let (_, _, emu, tx_stop) = Emulator::start(
            vec![("device:in".into(), rx_in)],
            vec![("device:out".into(), tx_out)],
            cfg,
        )
        .await
        .unwrap();

        // Send a value and see if it was forwarded.

        assert!(tx_in.send(device::Value::Int(1)).await.is_ok());

        let (value, rpy) = rx_out.recv().await.unwrap();
        let _ = rpy.send(Ok(value.clone()));

        assert_eq!(value, device::Value::Int(1));

        // Send the same value and see that it wasn't forwarded.

        assert!(tx_in.send(device::Value::Int(1)).await.is_ok());
        assert!(time::timeout(Duration::from_millis(100), rx_out.recv())
            .await
            .is_err());

        // Send a different value and see if it was forwarded.

        assert!(tx_in.send(device::Value::Int(9)).await.is_ok());

        let (value, rpy) = rx_out.recv().await.unwrap();
        let _ = rpy.send(Ok(value.clone()));

        assert_eq!(value, device::Value::Int(9));

        // Stop the emulator and see that its return status is good.

        let _ = tx_stop.send(());

        assert_eq!(emu.await.unwrap(), Ok(true));
    }

    // Test a basic logic block in which an input device's value is
    // used in a calculation and then forwarded to an output device.

    #[tokio::test]
    async fn test_basic_calc_node() {
        let cfg = build_config(
            &[("in", "device:in")],
            &[("out", "device:out")],
            &[],
            &["{in} > 5 -> {out}"],
        );
        let (tx_in, rx_in) = mpsc::channel(100);
        let (tx_out, mut rx_out) = mpsc::channel(100);

        let (_, _, emu, tx_stop) = Emulator::start(
            vec![("device:in".into(), rx_in)],
            vec![("device:out".into(), tx_out)],
            cfg,
        )
        .await
        .unwrap();

        // Send a value and see if it was forwarded.

        assert!(tx_in.send(device::Value::Int(10)).await.is_ok());

        let (value, rpy) = rx_out.recv().await.unwrap();
        let _ = rpy.send(Ok(value.clone()));

        assert_eq!(value, device::Value::Bool(true));

        // Send a different value that doesn't change the result and
        // see that it doesn't get forwarded.

        assert!(tx_in.send(device::Value::Int(9)).await.is_ok());
        assert!(time::timeout(Duration::from_millis(100), rx_out.recv())
            .await
            .is_err());

        // Send a different value and see if it was forwarded.

        assert!(tx_in.send(device::Value::Int(2)).await.is_ok());

        let (value, rpy) = rx_out.recv().await.unwrap();
        let _ = rpy.send(Ok(value.clone()));

        assert_eq!(value, device::Value::Bool(false));

        // Stop the emulator and see that its return status is good.

        let _ = tx_stop.send(());

        assert_eq!(emu.await.unwrap(), Ok(true));
    }

    // Test a basic logic block in which forwards a solar parameter to
    // a memory device.

    #[tokio::test]
    async fn test_basic_solar_node() {
        const OUT1: &str = "device:out1";
        const OUT2: &str = "device:out2";
        let cfg = build_config(
            &[],
            &[("alt", OUT1), ("dec", OUT2)],
            &[],
            &["{solar:alt} -> {alt}", "{solar:dec} -> {dec}"],
        );
        let (tx_out1, mut rx_out1) = mpsc::channel(100);
        let (tx_out2, mut rx_out2) = mpsc::channel(100);

        let (_, tx_solar, emu, tx_stop) = Emulator::start(
            vec![],
            vec![(OUT1.into(), tx_out1), (OUT2.into(), tx_out2)],
            cfg,
        )
        .await
        .unwrap();

        // Send a value and see if it was forwarded.

        assert!(tx_solar
            .send(Arc::new(solar::SolarInfo {
                elevation: 1.0,
                azimuth: 2.0,
                right_ascension: 3.0,
                declination: 4.0
            }))
            .is_ok());

        {
            let (value, rpy) = rx_out1.recv().await.unwrap();
            let _ = rpy.send(Ok(value.clone()));

            assert_eq!(value, device::Value::Flt(1.0));
        }

        {
            let (value, rpy) = rx_out2.recv().await.unwrap();
            let _ = rpy.send(Ok(value.clone()));

            assert_eq!(value, device::Value::Flt(4.0));
        }

        // Stop the emulator and see that its return status is good.

        let _ = tx_stop.send(());

        assert_eq!(emu.await.unwrap(), Ok(true));
    }

    // Test a logic block with two outputs. Make sure they are sent
    // "in parallel".

    #[tokio::test]
    async fn test_node_concurrency() {
        const IN1: &str = "device:in";
        const OUT1: &str = "device:out1";
        const OUT2: &str = "device:out2";

        // This section sets the output expressions in out1 -> out2
        // order. It computes a different value that is sent to the
        // outputs and verifies the correct value is sent to the
        // correct device.

        {
            let cfg = build_config(
                &[("in", IN1)],
                &[("out1", OUT1), ("out2", OUT2)],
                &[],
                &["{in} -> {out1}", "{in} * 2 -> {out2}"],
            );
            let (tx_in, rx_in) = mpsc::channel(100);
            let (tx_out1, mut rx_out1) = mpsc::channel(100);
            let (tx_out2, mut rx_out2) = mpsc::channel(100);

            let (_, _, emu, tx_stop) = Emulator::start(
                vec![(IN1.into(), rx_in)],
                vec![(OUT1.into(), tx_out1), (OUT2.into(), tx_out2)],
                cfg,
            )
            .await
            .unwrap();

            // Send a value and see if it was forwarded to both
            // channels. We hold off replying until we verify both
            // channels have content.

            assert!(tx_in.send(device::Value::Int(10)).await.is_ok());

            let (value1, rpy1) =
                time::timeout(Duration::from_millis(100), rx_out1.recv())
                    .await
                    .unwrap()
                    .unwrap();
            let (value2, rpy2) =
                time::timeout(Duration::from_millis(100), rx_out2.recv())
                    .await
                    .unwrap()
                    .unwrap();

            assert_eq!(value1, device::Value::Int(10));
            assert_eq!(value2, device::Value::Int(20));

            let _ = rpy1.send(Ok(value1.clone()));
            let _ = rpy2.send(Ok(value2.clone()));

            // Stop the emulator and see that its return status is good.

            let _ = tx_stop.send(());

            assert_eq!(emu.await.unwrap(), Ok(true));
        }

        // This section sets the output expressions in out2 -> out1
        // order. It computes a different value that is sent to the
        // outputs and verifies the correct value is sent to the
        // correct device.

        {
            let cfg = build_config(
                &[("in", IN1)],
                &[("out1", OUT1), ("out2", OUT2)],
                &[],
                &["{in} * 2 -> {out2}", "{in} -> {out1}"],
            );
            let (tx_in, rx_in) = mpsc::channel(100);
            let (tx_out1, mut rx_out1) = mpsc::channel(100);
            let (tx_out2, mut rx_out2) = mpsc::channel(100);

            let (_, _, emu, tx_stop) = Emulator::start(
                vec![(IN1.into(), rx_in)],
                vec![(OUT1.into(), tx_out1), (OUT2.into(), tx_out2)],
                cfg,
            )
            .await
            .unwrap();

            // Send a value and see if it was forwarded to both
            // channels. We hold off replying until we verify both
            // channels have content.

            assert!(tx_in.send(device::Value::Int(10)).await.is_ok());

            let (value1, rpy1) =
                time::timeout(Duration::from_millis(100), rx_out1.recv())
                    .await
                    .unwrap()
                    .unwrap();
            let (value2, rpy2) =
                time::timeout(Duration::from_millis(100), rx_out2.recv())
                    .await
                    .unwrap()
                    .unwrap();

            assert_eq!(value1, device::Value::Int(10));
            assert_eq!(value2, device::Value::Int(20));

            let _ = rpy1.send(Ok(value1.clone()));
            let _ = rpy2.send(Ok(value2.clone()));

            // Stop the emulator and see that its return status is good.

            let _ = tx_stop.send(());

            assert_eq!(emu.await.unwrap(), Ok(true));
        }

        // This section sets the output expressions in out2 -> out1
        // order. It computes a different value that is sent to the
        // outputs and verifies the correct value is sent to the
        // correct device. The expression uses some expression in the
        // `defs` section.

        {
            let cfg = build_config(
                &[("in", IN1)],
                &[("out1", OUT1), ("out2", OUT2)],
                &[("def1", "{in} * 10"), ("def2", "{in} * 100")],
                &[
                    "{def1} * 2 + {def2} + {in} -> {out2}",
                    "{def1} + {def2} * 2 + {in} * 3 -> {out1}",
                ],
            );
            let (tx_in, rx_in) = mpsc::channel(100);
            let (tx_out1, mut rx_out1) = mpsc::channel(100);
            let (tx_out2, mut rx_out2) = mpsc::channel(100);

            let (_, _, emu, tx_stop) = Emulator::start(
                vec![(IN1.into(), rx_in)],
                vec![(OUT1.into(), tx_out1), (OUT2.into(), tx_out2)],
                cfg,
            )
            .await
            .unwrap();

            // Send a value and see if it was forwarded to both
            // channels. We hold off replying until we verify both
            // channels have content.

            assert!(tx_in.send(device::Value::Int(4)).await.is_ok());

            let (value1, rpy1) =
                time::timeout(Duration::from_millis(100), rx_out1.recv())
                    .await
                    .unwrap()
                    .unwrap();
            let (value2, rpy2) =
                time::timeout(Duration::from_millis(100), rx_out2.recv())
                    .await
                    .unwrap()
                    .unwrap();

            assert_eq!(value1, device::Value::Int(852));
            assert_eq!(value2, device::Value::Int(484));

            let _ = rpy1.send(Ok(value1.clone()));
            let _ = rpy2.send(Ok(value2.clone()));

            // Stop the emulator and see that its return status is good.

            let _ = tx_stop.send(());

            assert_eq!(emu.await.unwrap(), Ok(true));
        }
    }
}
