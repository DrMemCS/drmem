use drmem_api::{client, device, driver, Error, Result};
use std::collections::HashMap;
use std::convert::Infallible;
use tokio::{sync::oneshot, task::JoinHandle};
use tokio_stream::{StreamExt, StreamMap};
use tracing::{debug, error, info, info_span};
use tracing_futures::Instrument;

use super::config;

mod compile;

// These are some helpful type aliases.

// The logic node will contain an array of these types. As readings
// come in, they'll be saved in this array.

type Inputs = Option<device::Value>;

// This is an array of channels in which settings are sent.

type Outputs = driver::TxDeviceSetting;

// This is a set of streams that receives all the readings.

type InputStream = StreamMap<usize, device::DataStream<device::Reading>>;

pub struct Node {
    inputs: Vec<Inputs>,
    outputs: Vec<Outputs>,
    in_stream: InputStream,
    // outputs: Vec<(String, )>,
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
        c_req: &client::RequestChan, vars: &HashMap<String, device::Name>,
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
        c_req: &client::RequestChan, vars: &HashMap<String, device::Name>,
    ) -> Result<(Vec<String>, Vec<driver::TxDeviceSetting>)> {
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

                    out_chans.push(ch);
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
        c_req: client::RequestChan, cfg: &config::Logic,
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

        // Return the initialized `Node`.

        Ok(Node {
            inputs: vec![None; inputs.len()],
            outputs: out_chans,
            in_stream,
            exprs: exprs?,
        })
    }

    // Runs the node logic. This method should never return.

    async fn run(&mut self) -> Result<Infallible> {
        info!("starting");

        // Wait for the next reading to arrive. All the incoming
        // streams have been combined into one and the returned value
        // is a pair consisting of an index and the actual reading.

        while let Some((idx, reading)) = self.in_stream.next().await {
            // Save the reading in our array for future
            // recalculations.

            self.inputs[idx] = Some(reading.value);

            // Loop through each defined expression.

            for compile::Program(expr, idx) in &self.exprs {
                // Compute the result of the expression, given the set
                // of inputs. If the result is None, then something in
                // the expression was wrong (either one or more input
                // values are None or the expression performed a bad
                // operation, like dividing by 0.)

                if let Some(result) = compile::eval(expr, &self.inputs) {
                    let (tx_rpy, rx_rpy) = oneshot::channel();

                    // Send the result to the correct setting channel.

                    if let Ok(()) =
                        self.outputs[*idx].send((result, tx_rpy)).await
                    {
                        // Wait for the result. It is possible that
                        // the driver returns a different value
                        // (clipping a setting, for instance.) We
                        // don't pay it any mind because the outer
                        // loop will receive the updated value.

                        if let Err(e) = rx_rpy.await {
                            error!("setting failed : {}", &e)
                        }
                    }
                }
            }
        }

        // This code should never be reached. If so, report the
        // problem and return an error.

        error!("all inputs have closed ... terminating");
        Err(Error::OperationError)
    }

    // Starts a new instance of a logic node.

    pub async fn start(
        c_req: client::RequestChan, cfg: &config::Logic,
    ) -> Result<JoinHandle<Result<Infallible>>> {
        let name = cfg.name.clone();

        // Create a new instance and let it initialize itself. If an
        // error occurs, return it.

        let mut node = Node::init(c_req, cfg)
            .instrument(info_span!("logic-init", name = &name))
            .await?;

        // Put the node in the background.

        Ok(tokio::spawn(async move {
            node.run().instrument(info_span!("logic", name)).await
        }))
    }
}
