use drmem_api::{client, device, driver, Result};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::{
    sync::{broadcast, oneshot},
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
    outputs: Vec<Output>,
    in_stream: InputStream,
    time_ch: Option<tod::TimeFilter>,
    solar_ch: Option<broadcast::Receiver<solar::Info>>,
    def_exprs: Vec<compile::Program>,
    exprs: Vec<compile::Program>,
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
                    inputs.push(vv.clone())
                }
                Err(e) => {
                    error!("error mapping '{}' to '{}': {}", &vv, &dev, &e);
                    return Err(e);
                }
            }
        }

        // Now add the definitions to the returned state. Definitions
        // add new variables to the vector of inputs.

        for (name, expr) in defs {
            // Make sure the name isn't already in the list of inputs.

            if inputs.iter().any(|v| v == name) {
                error!("definition tried to redefine {}", name);
                return Err(drmem_api::Error::ParseError(format!(
                    "can't redefine {}",
                    name
                )));
            }

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
            )?;

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
                    outputs.push(vv.clone())
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
        cfg: &config::Logic,
    ) -> Result<Node> {
        debug!("compiling expressions");

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
                Ok(ex) => info!("loaded : {}", &ex),
                Err(e) => error!("{}", &e),
            })
            .collect();
        let exprs = exprs?;

        // Look at each expression and see if it needs the
        // time-of-day.

        let needs_time = exprs
            .iter()
            .chain(&def_exprs)
            .filter_map(|compile::Program(e, _)| e.uses_time())
            .min();

        info!("needs time: {:?}", &needs_time);

        // Look at each expression and see if it needs any solar
        // information.

        let needs_solar = exprs
            .iter()
            .chain(&def_exprs)
            .any(|compile::Program(e, _)| e.uses_solar());

        info!("needs solar: {:?}", &needs_solar);

        // Return the initialized `Node`.

        Ok(Node {
            inputs: vec![None; inputs.len()],
            outputs: out_chans,
            in_stream,
            time_ch: needs_time
                .map(|tf| tod::time_filter(BroadcastStream::new(c_time), tf)),
            solar_ch: if needs_solar { Some(c_solar) } else { None },
            def_exprs,
            exprs,
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
            // will be `None` and we return a future that immediately
            // yields `None`.

            let wait_for_time = async {
                match self.time_ch.as_mut() {
                    None => None,
                    Some(s) => s.next().await,
                }
            };

            // Create a future that yields the next solar update. If
            // no expression uses solar data, `solar_ch` will be
            // `None` and we, instead, return a future that
            // immediately yields `None`.

            let wait_for_solar = async {
                match self.solar_ch.as_mut() {
                    None => None,
                    Some(ch) => ch.recv().await.ok(),
                }
            };

            #[rustfmt::skip]
	    tokio::select! {
		// Wait for the next reading to arrive. All the
		// incoming streams have been combined into one and
		// the returned value is a pair consisting of an index
		// and the actual reading.

		Some((idx, reading)) = self.in_stream.next() => {
		    info!("updating in[{}] with {}", idx, &reading.value);

		    // Save the reading in our array for future
		    // recalculations.

		    self.inputs[idx] = Some(reading.value);
		}

		// If we need the time channel, wait for the next
		// second.

		Some(v) = wait_for_time => {
		    info!("updating time");
		    time = v;
		}

		// If we need the solar channel, wait for the next
		// update.

		Some(v) = wait_for_solar => {
		    info!("updating solar position");
		    solar = Some(v);
		}
	    }

            // Calculate each expression of the `defs` array. Store
            // the result in the associated `input` cell.

            for compile::Program(expr, idx) in &self.def_exprs {
                self.inputs[*idx] =
                    compile::eval(expr, &self.inputs, &time, solar.as_ref());

                info!(
                    "def ({}) produced {:?} for in[{}]",
                    &expr, &self.inputs[*idx], *idx
                );
            }

            // Loop through each defined expression.

            for compile::Program(expr, idx) in &self.exprs {
                // Compute the result of the expression, given the set
                // of inputs. If the result is None, then something in
                // the expression was wrong (either one or more input
                // values are None or the expression performed a bad
                // operation, like dividing by 0.)

                if let Some(result) =
                    compile::eval(expr, &self.inputs, &time, solar.as_ref())
                {
                    info!(
                        "expr ({}) produced {:?} for out[{}]",
                        &expr, &result, *idx
                    );
                    let _ = self.outputs[*idx].send(result).await;
                } else {
                    error!("couldn't evaluate ({})", &expr)
                }
            }
        }
    }

    // Starts a new instance of a logic node.

    pub async fn start(
        c_req: client::RequestChan,
        rx_tod: broadcast::Receiver<tod::Info>,
        rx_solar: broadcast::Receiver<solar::Info>,
        cfg: &config::Logic,
    ) -> Result<JoinHandle<Result<Infallible>>> {
        let name = cfg.name.clone();

        // Create a new instance and let it initialize itself. If an
        // error occurs, return it.

        let node = Node::init(c_req, rx_tod, rx_solar, cfg)
            .instrument(info_span!("logic-init", name = &name))
            .await?;

        // Put the node in the background.

        Ok(tokio::spawn(async move {
            node.run().instrument(info_span!("logic", name)).await
        }))
    }
}

#[cfg(test)]
mod test {
    use super::{config, solar, tod, Node};
    use drmem_api::{
        client::{self, Request},
        device, Error, Result,
    };
    use futures::Future;
    use tokio::{
        sync::{broadcast, mpsc},
        task, try_join,
    };

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
        cfg: &'a config::Logic,
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
    async fn test_empty_node_init() {
        let cfg = build_config(&[], &[], &[], &[]);
        let (node, _, tod_tx, sol_tx) = init_node(&cfg);

        // `await` on the future. This should return immediately since
        // the config has no inputs or outputs.

        let node = node.await.unwrap();

        // With no config, the TOD and solar channel handles should
        // have been dropped.

        assert_eq!(tod_tx.receiver_count(), 0);
        assert_eq!(sol_tx.receiver_count(), 0);

        // This call allows us to keep ownership of the node so we're
        // sure the Node dropped the broadcast receivers and not from
        // an early drop of `node` itself.

        std::mem::drop(node);
    }

    #[tokio::test]
    async fn test_node_initialization() {
        let cfg = build_config(
            &[],
            &[("out", "device:out")],
            &[],
            &["{utc:second} -> {out}"],
        );
        let (node_fut, mut req_rx, _, _) = init_node(&cfg);

        // Run the initialization concurrently with handling the one
        // request it is going to make.

        #[rustfmt::skip]
	try_join!(
	    node_fut,
	    async {
		match (&mut req_rx).recv().await.unwrap() {
		    Request::GetSettingChan { name, rpy_chan, .. } => {
			if name.to_string() == "device:out" {
			    let (tx, _) = mpsc::channel(10);

			    assert!(rpy_chan.send(Ok(tx)).is_ok());
			} else {
			    assert!(
				rpy_chan.send(Err(Error::NotFound)).is_ok());
			}
		    }
		    Request::QueryDeviceInfo { rpy_chan, .. } =>
			assert!(rpy_chan
				.send(Err(Error::ProtocolError(
				    "bad request".into())))
				.is_ok()),
		    Request::SetDevice { rpy_chan, .. } =>
			assert!(rpy_chan
				.send(Err(Error::ProtocolError(
				    "bad request".into())))
				.is_ok()),
		    Request::MonitorDevice { rpy_chan, .. } =>
			assert!(rpy_chan
				.send(Err(Error::ProtocolError(
				    "bad request".into())))
				.is_ok())
		};
		Ok(())
	    }
	)
	.unwrap();
    }
}
