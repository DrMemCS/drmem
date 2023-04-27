use drmem_api::{client, types::Error, Result};
use futures::future;
use std::convert::Infallible;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, info_span};
use tracing_futures::Instrument;

use super::config;

mod compile;

pub struct Node {
    notes: Option<String>,
    exprs: Vec<compile::Program>,
}

impl Node {
    // Creates an instance of `Node` and initializes its state using
    // the configuration information.

    async fn init(
        c_req: client::RequestChan, cfg: &config::Logic,
    ) -> Result<Node> {
        debug!("compiling expressions");

        let exprs: Result<Vec<compile::Program>> = cfg
            .exprs
            .iter()
            .map(|s| compile::compile(s.as_str()))
            .inspect(|e| match e {
                Ok(ex) => info!("loaded : {}", &ex),
                Err(e) => error!("{}", &e),
            })
            .collect();

        Ok(Node {
            notes: cfg.summary.clone(),
            exprs: exprs?,
        })
    }

    // Runs the node logic. This method should never return.

    async fn run(&mut self) -> Result<Infallible> {
        info!("starting");
        future::pending().await
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
