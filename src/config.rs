use tracing::{Level, info};
use tracing_subscriber;

// Holds the configuration for the running system. Its global method,
// determine, reads the config file and parses the command line to
// determine the final configuration.

pub struct Config {
    redis_addr: String,
    redis_port: u16
}

//fn find_config() -> Option<String> {
//    if
//}

impl Config {
    pub fn get_redis_addr<'a>(&'a self) -> &'a String { &self.redis_addr }

    pub fn get_redis_port(&self) -> u16 { self.redis_port }

    pub fn determine() -> Option<Config> {
	use clap::{Arg, App};

	// Define the command line arguments.

	let matches = App::new("DrMemory Mini Control System")
            .version("0.1")
            .author("Rich Neswold <rich.neswold@gmail.com>")
            .about("A small, yet capable, control system.")
            .arg(Arg::with_name("config")
		 .short("c")
		 .long("config")
		 .value_name("FILE")
		 .help("Specifies the configuration file")
		 .takes_value(true))
            .arg(Arg::with_name("db_addr")
		 .short("A")
		 .long("db_addr")
		 .value_name("ADDR")
		 .help("IP address of redis database; defaults to localhost")
		 .takes_value(true))
            .arg(Arg::with_name("db_port")
		 .short("P")
		 .long("db_port")
		 .value_name("PORT")
		 .help("IP port address of redis database; defaults to 6379")
		 .takes_value(true))
            .arg(Arg::with_name("verbose")
		 .short("v")
		 .long("verbose")
		 .multiple(true)
		 .help("Sets verbosity of log; can be used more than once")
		 .takes_value(false))
            .arg(Arg::with_name("print_cfg")
		 .long("print-config")
		 .help("Displays the configuration and exits")
		 .takes_value(false))
            .get_matches();

	// Return the number of '-v' options to determine the log
	// level.

	let level = match matches.occurrences_of("verbose") {
            0 => Level::WARN,
            1 => Level::INFO,
            2 => Level::DEBUG,
	    _ => Level::TRACE
	};

	// Initialize the log system. The max log level is determined
	// by the user (either through the config file or the command
	// line.)

	let subscriber = tracing_subscriber::fmt()
	    .with_max_level(level.clone())
	    .finish();

	tracing::subscriber::set_global_default(subscriber)
	    .expect("Unable to set global default subscriber");

	// Generate the configuration based on the command line arguments.

	let cfg = Config { redis_addr: matches.value_of("db_addr")
			   .unwrap_or("127.0.0.1")
			   .to_string(),
			   redis_port: matches.value_of("db_port")
			   .unwrap_or("6379")
			   .parse::<u16>()
			   .expect("port number should be an integer") };

	// If the user just wants the config printed out, do it and
	// then exit (by returning `None`.)

	if matches.is_present("print_cfg") {
	    print!("redis address: {}:{}\n", &cfg.redis_addr, &cfg.redis_port);
	    print!("Log level: {}\n", level);
	    None
	} else {
	    info!("logging level set to {}", level);
	    Some(cfg)
	}
    }
}
