use drmem_api::{
    driver::{self, API},
    Result,
};
use drmem_config::Config;
use futures::future;
use tokio::task::JoinHandle;
use tracing::error;

mod core;

// If the user specifies the 'grpc' feature, then pull in the module
// that defines the gRPC server.

#[cfg(feature = "grpc")]
mod grpc;

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
            .finish();

        tracing::subscriber::set_global_default(subscriber)
            .expect("Unable to set global default subscriber");
        Some(cfg)
    } else {
        None
    }
}

async fn wrap_task(handle: JoinHandle<Result<()>>) -> Result<()> {
    match handle.await {
	Err(e) if e.is_panic() => error!("terminated due to panic"),
	Err(_) => error!("terminated due to cancellation"),
	Ok(Err(e)) => error!("task returned error -- {}", &e),
	Ok(Ok(_)) => ()
    }
    Ok(())
}

// Runs the main body of the application. This top-level task reads
// the config, starts the drivers, and monitors their health.

async fn run() -> Result<()> {
    if let Some(_cfg) = init_app().await {
        // Start the core task. It returns a handle to a channel with
        // which to make requests. It also returns the task handle.

        let (tx_drv_req, core_task) = core::start().await?;

        let mut drv_pump = drmem_drv_sump::Sump::create_instance(
            &driver::Config::new(),
            driver::RequestChan::new("basement:sump", &tx_drv_req),
        )
        .await?;

        let _ = future::join_all(vec![
            wrap_task(core_task),
            #[cfg(feature = "graphql")]
            wrap_task(tokio::spawn(graphql::server())),
            wrap_task(tokio::spawn(async move { drv_pump.run().await })),
        ]);
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("ERROR: {:?}", e)
    }
}
