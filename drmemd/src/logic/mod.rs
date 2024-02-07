use drmem_api::{client, device, driver, Result};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::{
    sync::{broadcast, oneshot},
    task::JoinHandle,
};
use tokio_stream::{StreamExt, StreamMap};
use tracing::{debug, error, info, info_span, warn};
use tracing_futures::Instrument;

use super::config;

mod compile;
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

    pub async fn send(&mut self, value: device::Value) {
        // Only attempt the setting if the device isn't set to the
        // value.

        if self.prev.as_ref() != Some(&value) {
            let (tx_rpy, rx_rpy) = oneshot::channel();

            if let Ok(()) = self.chan.send((value.clone(), tx_rpy)).await {
                match rx_rpy.await {
                    Ok(Ok(v)) => {
                        // Save the value returned by the driver. This
                        // should, hopefully, be the same that we
                        // sent. If it isn't, add a warning to the
                        // log.

                        self.prev = Some(v.clone());
                        if v != value {
                            warn!("setting was adjusted")
                        }
                    }
                    Ok(Err(e)) => error!("driver rejected setting : {}", &e),
                    Err(e) => error!("setting failed : {}", &e),
                }
            } else {
                error!("driver not accepting settings")
            }
        }
    }
}

pub struct Node {
    inputs: Vec<Inputs>,
    outputs: Vec<Output>,
    in_stream: InputStream,
    time_ch: Option<broadcast::Receiver<tod::Info>>,
    exprs: Vec<compile::Program>,
}

impl Node {
    // Iterate through the input device mapping. As we work through
    // the list, build two things:
    //
    // 1) An array of pairs where the first element is the variable
    // name and the second element is the current value.
    //
    // 2) A chained set of streams which provide the readings.

    async fn setup_inputs(
        c_req: &client::RequestChan,
        vars: &HashMap<String, device::Name>,
    ) -> Result<(Vec<String>, InputStream)> {
        let mut inputs = Vec::with_capacity(vars.len());
        let mut in_stream = StreamMap::with_capacity(vars.len());

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
        Ok((inputs, in_stream))
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
        cfg: &config::Logic,
    ) -> Result<Node> {
        debug!("compiling expressions");

        let (inputs, in_stream) =
            Node::setup_inputs(&c_req, &cfg.inputs).await?;

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

        // Look at each expression and see if it needs the time-of-day.

        let needs_time =
            exprs.iter().any(|compile::Program(e, _)| e.uses_time());

        // Return the initialized `Node`.

        Ok(Node {
            inputs: vec![None; inputs.len()],
            outputs: out_chans,
            in_stream,
            time_ch: if needs_time { Some(c_time) } else { None },
            exprs,
        })
    }

    // Runs the node logic. This method should never return.

    async fn run(mut self) -> Result<Infallible> {
        let mut time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        info!("starting");

        loop {
            #[rustfmt::skip]
	    tokio::select! {
		// Wait for the next reading to arrive. All the
		// incoming streams have been combined into one and
		// the returned value is a pair consisting of an index
		// and the actual reading.

		Some((idx, reading)) = self.in_stream.next() => {
		    // Save the reading in our array for future
		    // recalculations.

		    self.inputs[idx] = Some(reading.value);
		    debug!("updated input[{}]", idx);
		}

		// If we need the time channel, wait for the next
		// second.

		Ok(v) = self.time_ch.as_mut().unwrap().recv(),
		            if self.time_ch.is_some() => {
		    time = v;
		    debug!("updated time");
		}
	    }

            // Loop through each defined expression.

            for compile::Program(expr, idx) in &self.exprs {
                // Compute the result of the expression, given the set
                // of inputs. If the result is None, then something in
                // the expression was wrong (either one or more input
                // values are None or the expression performed a bad
                // operation, like dividing by 0.)

                if let Some(result) = compile::eval(expr, &self.inputs, &time) {
                    self.outputs[*idx].send(result).await
                } else {
                    error!("couldn't evaluate {}", &expr)
                }
            }
        }
    }

    // Starts a new instance of a logic node.

    pub async fn start(
        c_req: client::RequestChan,
        rx_tod: broadcast::Receiver<tod::Info>,
        cfg: &config::Logic,
    ) -> Result<JoinHandle<Result<Infallible>>> {
        let name = cfg.name.clone();

        // Create a new instance and let it initialize itself. If an
        // error occurs, return it.

        let node = Node::init(c_req, rx_tod, cfg)
            .instrument(info_span!("logic-init", name = &name))
            .await?;

        // Put the node in the background.

        Ok(tokio::spawn(async move {
            node.run().instrument(info_span!("logic", name)).await
        }))
    }
}
