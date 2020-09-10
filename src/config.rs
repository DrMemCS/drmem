use tracing::Level;

pub struct Config(toml::Value);

impl Config {
    pub fn redis_addr(&self) -> String {
	if let Some(tbl) = self.0.get("redis") {
	    if let Some(val) = tbl.get("addr") {
		if let toml::Value::String(addr) = val {
		    return addr.clone();
		}
	    }
	}
	"127.0.0.1".to_string()
    }

    pub fn redis_port(&self) -> u16 {
	if let Some(tbl) = self.0.get("redis") {
	    if let Some(val) = tbl.get("port") {
		if let toml::Value::Integer(port) = val {
		    return *port as u16;
		}
	    }
	}
	6379u16
    }

    pub fn log_level(&self) -> Level {
	match self.0["log_level"].as_str() {
	    Some("trace") => Level::TRACE,
	    Some("debug") => Level::DEBUG,
	    Some("info") => Level::INFO,
	    Some("error") => Level::ERROR,
	    _ => Level::WARN
	}
    }
}

pub fn from_cmdline() -> (bool, Config) {
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

    // Generate the configuration based on the command line arguments.

    use toml::Value;

    let log_level_key = "log_level";
    let addr_key = "addr";
    let redis_key = "redis";
    let port_key = "port";

    let mut cfg = toml::toml! {
	log_level = "warn"

	[hue_bridge]
	key = ""
	addr = ""

	[redis]
	addr = "127.0.0.1"
	port = 6379
    };

    if let Some(addr) = matches.value_of("db_addr") {
	cfg[redis_key][addr_key] = Value::String(addr.to_string());
    }

    if let Some(port) = matches.value_of("db_port") {
	cfg[redis_key][port_key] =
	    Value::Integer(port.parse::<u16>().expect("bad port value") as i64);
    }

    // The number of '-v' options determines the log level.

    cfg[log_level_key] = match matches.occurrences_of("verbose") {
        0 => Value::String("warn".to_string()),
        1 => Value::String("info".to_string()),
        2 => Value::String("debug".to_string()),
	_ => Value::String("trace".to_string())
    };

    // Return the config built from the command line and a flag
    // indicating the user wants the final configuration displayed.

    (matches.is_present("print_cfg"), Config(cfg))
}

pub fn get() -> Option<Config> {
    let (print_cfg, cfg) = from_cmdline();

    if print_cfg {
	println!("redis address: {}:{}", cfg.redis_addr(), cfg.redis_port());
	println!("Log level: {}", cfg.log_level());
	None
    } else {
	Some(cfg)
    }
}
