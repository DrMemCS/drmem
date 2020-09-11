use tracing::Level;
use serde_derive::{Serialize,Deserialize};

#[derive(Serialize,Deserialize)]
pub struct Config {
    log_level: String,
    pub redis: Redis,
    pub hue_bridge: HueBridge
}

impl Config {
    pub fn get_log_level(&self) -> Level {
	match self.log_level.as_str() {
	    "info" => Level::INFO,
	    "debug" => Level::DEBUG,
	    "trace" => Level::TRACE,
	    _ => Level::WARN
	}
    }
}

impl Default for Config {
    fn default() -> Self {
	Config {
	    log_level: "warn".to_string(),
	    redis: Redis::default(),
	    hue_bridge: HueBridge::default()
	}
    }
}

#[derive(Serialize,Deserialize)]
pub struct Redis {
    pub addr: String,
    pub port: u16
}

impl Default for Redis {
    fn default() -> Self {
	Redis {
	    addr: "127.0.0.1".to_string(),
	    port: 6379
	}
    }
}

#[derive(Serialize,Deserialize)]
pub struct HueBridge {
    pub addr: String,
    pub key: Option<String>
}

impl Default for HueBridge {
    fn default() -> Self {
	HueBridge {
	    addr: "10.0.0.1".to_string(),
	    key: None
	}
    }
}

fn from_cmdline(mut cfg: Config) -> (bool, Config) {
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

    if let Some(addr) = matches.value_of("db_addr") {
	cfg.redis.addr = addr.to_string()
    }

    if let Some(port) = matches.value_of("db_port") {
	if let Ok(port) = port.parse::<u16>() {
	    cfg.redis.port = port
	}
    }

    // The number of '-v' options determines the log level.

    match matches.occurrences_of("verbose") {
        0 => (),
        1 => cfg.log_level = "info".to_string(),
        2 => cfg.log_level = "debug".to_string(),
	_ => cfg.log_level = "trace".to_string()
    };

    // Return the config built from the command line and a flag
    // indicating the user wants the final configuration displayed.

    (matches.is_present("print_cfg"), cfg)
}

async fn from_file(path: &str) -> Option<Config> {
    use tokio::fs;

    if let Ok(contents) = fs::read(path).await {
	let contents = String::from_utf8_lossy(&contents);

	if let Ok(cfg) = toml::from_str(&contents) {
	    return Some(cfg)
	} else {
	    println!("error parsing {}", path);
	}
    }
    None
}

async fn find_cfg() -> Config {
    if let Some(cfg) = from_file("./drmem.conf").await {
	cfg
    } else {
	use std::env;

	if let Ok(home) = env::var("HOME") {
	    if let Some(cfg) = from_file(&(home + "/.drmem.conf")).await {
		return cfg;
	    }
	}
	if let Some(cfg) = from_file("/usr/local/etc/drmem.conf").await {
	    cfg
	} else if let Some(cfg) = from_file("/usr/pkg/etc/drmem.conf").await {
	    cfg
	} else if let Some(cfg) = from_file("/etc/drmem.conf").await {
	    cfg
	} else {
	    Config::default()
	}
    }
}

pub async fn get() -> Option<Config> {
    let cfg = find_cfg().await;
    let (print_cfg, cfg) = from_cmdline(cfg);

    if print_cfg {
	match toml::to_string(&cfg) {
	    Ok(s) => println!("Combined configuration:\n\n{}", s),
	    Err(e) => println!("Configuration error: {:?}", e)
	}
	None
    } else {
	Some(cfg)
    }
}
