use drmem_api::{driver::RequestChan, types::Error, Result};
use drmem_config::Config;
use futures::{future, FutureExt};
use std::convert::Infallible;
use tokio::task::JoinHandle;
use tracing::{error, trace, warn};

mod core;
mod driver;

// If the user specifies the 'graphql' feature, then pull in the module
// that defines the GraphQL server.

#[cfg(feature = "graphql")]
mod graphql;

// Initializes the `drmemd` application. It determines the
// configuration and sets up the logger. It returns `Some(Config)`
// with the found configuration, if the applications is to run. It
// returns `None` if the program should exit (because a command line
// option asked for a "usage" message, for instance.)

async fn init_app() -> Option<Config> {
    // If a configuration is returned, set up the logger.

    if let Some(cfg) = drmem_config::get().await {
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
            Err(Error::OperationError)
        }

        Err(_) => {
            error!("terminated due to cancellation");
            Err(Error::OperationError)
        }

        Ok(Ok(_)) => unreachable!(),

        Ok(Err(e)) => {
            error!("task returned error -- {}", &e);
            Err(e)
        }
    }
}

// Runs the main body of the application. This top-level task reads
// the config, starts the drivers, and monitors their health.

async fn run() -> Result<()> {
    if let Some(cfg) = init_app().await {
        let drv_tbl = driver::DriverDb::create();

        // Start the core task. It returns a handle to a channel with
        // which to make requests. It also returns the task handle.

        let (tx_drv_req, tx_clnt_req, core_task) = core::start(&cfg).await?;

        trace!("starting core tasks");

        // Build initial vector of required tasks. Crate features will
        // enable more required tasks.

        let mut tasks = vec![wrap_task(core_task)];

        #[cfg(feature = "graphql")]
        {
            let f = graphql::server(
		&cfg.get_name(),
                &cfg.get_graphql_addr(),
                drv_tbl.clone(),
                tx_clnt_req.clone(),
            )
            .then(|_| async { Err(Error::OperationError) });

            tasks.push(wrap_task(tokio::spawn(f)));
        }

        // Iterate through the list of drivers specified in the
        // configuration file.

        trace!("starting driver instances");

        for driver in cfg.driver {
            let driver_name = driver.name.to_string();

            // If the driver exists in the driver table, an instance
            // can be started. If it doesn't exist, report an error
            // and exit.

            if let Some(driver_info) = drv_tbl.get_driver(&driver_name) {
                let chan =
                    RequestChan::new(&driver_name, &driver.prefix, &tx_drv_req);
                let instance = driver_info.run_instance(
                    driver_name,
                    driver.max_history,
                    driver.cfg.unwrap_or_default().clone(),
                    chan,
                );

                tasks.push(wrap_task(instance))
            } else {
                error!("no driver named {}", driver.name);
                return Err(Error::NotFound);
            }
        }

        let _ = future::join_all(tasks).await;

        warn!("shutting down")
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("ERROR: {:?}", e)
    }
}
