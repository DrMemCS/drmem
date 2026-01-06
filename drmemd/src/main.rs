#![deny(unsafe_code)]

#[cfg(feature = "graphql")]
#[macro_use]
extern crate lazy_static;

use drmem_api::{driver::RequestChan, Error, Result};
use futures::future;
use std::{convert::Infallible, sync::Arc};
use tokio::{sync::Barrier, task::JoinHandle};
use tracing::{error, info, info_span, warn, Instrument};

mod config;
mod core;
mod driver;
mod logic;

pub mod backends;

// If the user specifies the 'graphql' feature, then pull in the module
// that defines the GraphQL server.

#[cfg(feature = "graphql")]
mod graphql;

// Initializes the `drmemd` application. It determines the
// configuration and sets up the logger. It returns `Some(Config)`
// with the found configuration, if the applications is to run. It
// returns `None` if the program should exit (because a command line
// option asked for a "usage" message, for instance.)

async fn init_app() -> Option<config::Config> {
    // If a configuration is returned, set up the logger.

    if let Some(cfg) = config::get().await {
        // Initialize the log system. The max log level is determined
        // by the user (either through the config file or the command
        // line.)

        let subscriber = tracing_subscriber::fmt()
            .with_max_level(cfg.get_log_level())
            .with_target(false)
            .finish();

        tracing::subscriber::set_global_default(subscriber)
            .expect("Unable to set global default subscriber");
        Some(cfg)
    } else {
        None
    }
}

async fn wrap_task(
    handle: JoinHandle<Result<Infallible>>,
) -> Result<Infallible> {
    match handle.await {
        Err(e) if e.is_panic() => {
            error!("terminated due to panic");
            Err(Error::OperationError("task panicked".to_owned()))
        }

        Err(_) => {
            error!("terminated due to cancellation");
            Err(Error::OperationError("task was canceled".to_owned()))
        }

        Ok(Ok(_)) => unreachable!(),

        Ok(Err(e)) => {
            error!("task returned error -- {}", &e);
            Err(e)
        }
    }
}

// Runs the main body of the application. This top-level task reads
// the config, starts the drivers and logic node, and monitors their
// health.

async fn run() -> Result<()> {
    if let Some(cfg) = init_app().await {
        let drv_tbl = driver::DriverDb::create();

        // Start the core task. It returns a handle to a channel with
        // which to make requests. It also returns the task handle.

        let (tx_drv_req, tx_clnt_req, core_task) = core::start(&cfg).await?;

        // Build initial vector of required tasks. Crate features will
        // enable more required tasks.

        let mut tasks = vec![wrap_task(core_task)];

        // Iterate through the list of drivers specified in the
        // configuration file.

        info!("starting drivers");

        for driver in cfg.driver {
            let driver_name: drmem_api::driver::Name =
                driver.name.clone().into();

            // If the driver exists in the driver table, an instance
            // can be started. If it doesn't exist, report an error
            // and exit.

            if let Some(driver_info) = drv_tbl.get_driver(&driver_name) {
                let chan = RequestChan::new(
                    driver_name.clone(),
                    &driver.prefix,
                    &tx_drv_req,
                );
                let barrier = Arc::new(Barrier::new(2));

                // Call the function that manages instances of this
                // driver. If it returns `Ok()`, the value is a Future
                // that implements the driver. If `Err()` is returned,
                // then the devices couldn't be registered or some
                // other serious error occurred.

                let instance = (driver_info.2)(
                    driver.cfg.unwrap_or_default().clone(),
                    chan,
                    driver.max_history,
                    barrier.clone(),
                );

                // Push the driver instance at the end of the vector.

                tasks.push(wrap_task(tokio::spawn(instance.instrument(
                    info_span!(
                        "driver",
                        name = driver_name.as_ref(),
                        prefix = driver.prefix.to_string()
                    ),
                ))));

                let _ = barrier.wait().await;
            } else {
                error!("no driver named {}", driver.name);
                return Err(Error::NotFound);
            }
        }

        // Create a nested scope so that the tod and solar handles are
        // freed up.

        {
            let barrier = Arc::new(Barrier::new(3 + cfg.logic.len()));

            // Start the time-of-day task. This needs to be done
            // *before* any logic blocks are started because logic
            // blocks *may* have an expression that uses the
            // time-of-day.

            let (tx_tod, _) = logic::tod::create_task(barrier.clone());

            // Start the solar task. This, too, needs to be done
            // before any logic blocks are started.

            let (tx_solar, _) = logic::solar::create_task(
                cfg.latitude,
                cfg.longitude,
                barrier.clone(),
            );

            info!("starting logic blocks");

            // Iterate through the [[logic]] sections of the config.

            for logic in cfg.logic.iter() {
                let node_task = logic::Node::start(
                    tx_clnt_req.clone(),
                    tx_tod.subscribe(),
                    tx_solar.subscribe(),
                    logic.clone(),
                    barrier.clone(),
                )
                .await;

                tasks.push(wrap_task(node_task));
            }

            // Now that all the logic blocks have initialized, we
            // start the tasks blocked by the Barrier.

            let _ = barrier.wait().await;
        }

        // If the "graphql" feature is specified, start up the web
        // server which accepts GraphQL queries. We do this last so
        // that all the queries will have access to all the
        // drivers/blocks/devices that were created above.

        #[cfg(feature = "graphql")]
        {
            use futures::FutureExt;

            info!("starting GraphQL");

            // This server should never exit. If it does, report an
            // `OperationError`,

            let f = graphql::server(
                &cfg.graphql,
                &cfg.logic,
                drv_tbl.clone(),
                tx_clnt_req.clone(),
            )
            .instrument(info_span!("gql"))
            .then(|_| async {
                Err(Error::OperationError("graphql server exited".to_owned()))
            });

            tasks.push(wrap_task(tokio::spawn(f)));
        }

        // Now run all the tasks.

        let _ = future::join_all(tasks).await;

        warn!("shutting down")
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("ERROR: {e:?}")
    }
}
