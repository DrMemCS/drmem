use super::store;
use drmem_api::{Error, Result};
use serde_derive::Deserialize;
use std::collections::HashMap;
use std::env;
use toml::{self, value};
use tracing::Level;

use drmem_api::{device, driver::DriverConfig};

fn def_log_level() -> String {
    String::from("warn")
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "def_log_level")]
    log_level: String,
    pub latitude: f64,
    pub longitude: f64,
    #[cfg(feature = "graphql")]
    #[serde(default)]
    pub graphql: super::graphql::config::Config,
    pub backend: Option<store::config::Config>,
    #[serde(default)]
    pub driver: Vec<Driver>,
    #[serde(default)]
    pub logic: Vec<Logic>,
}

impl<'a> Config {
    pub fn get_log_level(&self) -> Level {
        match self.log_level.as_str() {
            "info" => Level::INFO,
            "debug" => Level::DEBUG,
            "trace" => Level::TRACE,
            _ => Level::WARN,
        }
    }

    pub fn get_backend(&'a self) -> &'a store::config::Config {
        self.backend.as_ref().unwrap_or(&store::config::DEF)
    }

    #[cfg(feature = "graphql")]
    pub fn get_graphql_addr(&self) -> std::net::SocketAddr {
        self.graphql.addr
    }

    #[cfg(feature = "graphql")]
    pub fn get_name(&self) -> String {
        self.graphql.name.clone()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            log_level: String::from("warn"),
            latitude: 0.0,
            longitude: 0.0,
            #[cfg(feature = "graphql")]
            graphql: super::graphql::config::Config::default(),
            backend: Some(store::config::Config::new()),
            driver: vec![],
            logic: vec![],
        }
    }
}

#[derive(Deserialize)]
pub struct Driver {
    pub name: String,
    pub prefix: device::Path,
    pub max_history: Option<usize>,
    pub cfg: Option<DriverConfig>,
}

#[derive(Deserialize)]
pub struct Logic {
    pub name: String,
    pub summary: Option<String>,
    #[serde(default)]
    pub defs: HashMap<String, String>,
    pub exprs: Vec<String>,
    #[serde(default)]
    pub inputs: HashMap<String, device::Name>,
    pub outputs: HashMap<String, device::Name>,
}

fn from_cmdline(mut cfg: Config) -> (bool, Config) {
    use clap::{crate_version, Arg, ArgAction, Command};

    // Define the command line arguments.

    let matches = Command::new("DrMem Mini Control System")
        .version(crate_version!())
        .about("A small, yet capable, control system.")
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .action(ArgAction::Set)
                .value_name("FILE")
                .help("Specifies the configuration file"),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .action(ArgAction::Count)
                .help("Sets verbosity of log; can be used more than once"),
        )
        .arg(
            Arg::new("print_cfg")
                .long("print-config")
                .action(ArgAction::SetTrue)
                .help("Displays the configuration and exits"),
        )
        .get_matches();

    // The number of '-v' options determines the log level.

    match matches.get_count("verbose") {
        0 => (),
        1 => cfg.log_level = String::from("info"),
        2 => cfg.log_level = String::from("debug"),
        _ => cfg.log_level = String::from("trace"),
    };

    // Return the config built from the command line and a flag
    // indicating the user wants the final configuration displayed.

    (matches.get_flag("print_cfg"), cfg)
}

fn parse_config(contents: &str) -> Result<Config> {
    toml::from_str(contents)
        .map_err(|e| Error::ConfigError(format!("{}", e)))
        .and_then(|cfg: Config| {
            // Make sure latitude is between -90 and 90 degrees.

            if !(-90.0..=90.0).contains(&cfg.latitude) {
                return Err(Error::ConfigError(
                    "'latitude' is out of range".into(),
                ));
            }

            // Make sure longitude is between -180 and 180 degrees.

            if !(-180.0..=180.0).contains(&cfg.longitude) {
                return Err(Error::ConfigError(
                    "'longitude' is out of range".into(),
                ));
            }
            Ok(cfg)
        })
}

async fn from_file(path: &str) -> Option<Result<Config>> {
    use tokio::fs;

    if let Ok(contents) = fs::read(path).await {
        let contents = String::from_utf8_lossy(&contents);

        Some(parse_config(&contents))
    } else {
        None
    }
}

async fn find_cfg() -> Result<Config> {
    const CFG_FILE: &str = "drmem.toml";

    // Create a vector of directories that could contain a
    // configuration file. The directories will be searched in their
    // order within the vector.

    let mut dirs = vec![String::from("./")];

    // If the user has `HOME` defined, append their home directory to
    // the search path. Note the end of the path has a period. This is
    // done so the file will be named `.drmem.toml` in the home
    // directory. (Kind of hack-y, I know.)

    if let Ok(home) = env::var("HOME") {
        dirs.push(format!("{}/.", home))
    }

    // Add other, common configuration areas.

    dirs.push(String::from("/usr/local/etc/"));
    dirs.push(String::from("/usr/pkg/etc/"));
    dirs.push(String::from("/etc/"));

    // Iterate through the directories. The first file that is found
    // and can be parsed is used as the configuration.

    for dir in dirs {
        let file = format!("{}{}", &dir, CFG_FILE);

        if let Some(cfg) = from_file(&file).await {
            return cfg;
        }
    }
    Ok(Config::default())
}

fn dump_config(cfg: &Config) {
    println!("Configuration:");
    println!("    log level: {}\n", cfg.get_log_level());

    #[cfg(feature = "simple-backend")]
    {
        println!("Using SIMPLE backend -- no configuration for it.\n");
    }

    #[cfg(feature = "redis-backend")]
    {
        println!("Using REDIS for storage:");
        println!("    address: {}", &cfg.get_backend().get_addr());
        println!("    db #: {}\n", cfg.get_backend().get_dbn());
    }

    #[cfg(feature = "graphql")]
    {
        println!("Using GraphQL:");
        println!("    instance name: {}", cfg.get_name());
        println!("    address: {}\n", cfg.get_graphql_addr());
    }

    println!("Driver configuration:");
    if !cfg.driver.is_empty() {
        for ii in &cfg.driver {
            println!(
                "    name: {}\n    prefix: '{}'\n    cfg: {:?}\n",
                &ii.name,
                &ii.prefix,
                ii.cfg.as_ref().unwrap_or(&value::Table::new())
            )
        }
    } else {
        println!("    No drivers specified.");
    }
}

#[tracing::instrument(name = "loading config")]
pub async fn get() -> Option<Config> {
    match find_cfg().await {
        Ok(cfg) => {
            let (print_cfg, cfg) = from_cmdline(cfg);

            if print_cfg {
                dump_config(&cfg);
                None
            } else {
                Some(cfg)
            }
        }
        Err(e) => {
            println!("{}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config() {
        // Verify the defaults.

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0
"#,
        ) {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::WARN),
            Err(e) => panic!("TOML parse error: {}", e),
        }

        // Verify the log_level can be set.

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0
log_level = "trace"
"#,
        ) {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::TRACE),
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0
log_level = "debug"
"#,
        ) {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::DEBUG),
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0
log_level = "info"
"#,
        ) {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::INFO),
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0
log_level = "warn"
"#,
        ) {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::WARN),
            Err(e) => panic!("TOML parse error: {}", e),
        }
    }

    #[cfg(feature = "graphql")]
    #[test]
    fn test_graphql_config() {
        use std::net::Ipv4Addr;

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.graphql.name, "unknown name");
                assert_eq!(cfg.graphql.location, "unknown location");
                assert_eq!(
                    cfg.graphql.addr,
                    (Ipv4Addr::new(0, 0, 0, 0), 3000).into()
                );
                assert_eq!(cfg.graphql.pref_host, None);
                assert_eq!(cfg.graphql.pref_port, 3000)
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[graphql]"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.graphql.name, "unknown name");
                assert_eq!(cfg.graphql.location, "unknown location");
                assert_eq!(
                    cfg.graphql.addr,
                    (Ipv4Addr::new(0, 0, 0, 0), 3000).into()
                );
                assert_eq!(cfg.graphql.pref_host, None);
                assert_eq!(cfg.graphql.pref_port, 3000)
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[graphql]
name = "primary-node"
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.graphql.name, "primary-node");
                assert_eq!(cfg.graphql.location, "unknown location");
                assert_eq!(
                    cfg.graphql.addr,
                    (Ipv4Addr::new(0, 0, 0, 0), 3000).into()
                );
                assert_eq!(cfg.graphql.pref_host, None);
                assert_eq!(cfg.graphql.pref_port, 3000)
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[graphql]
location = "basement"
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.graphql.name, "unknown name");
                assert_eq!(cfg.graphql.location, "basement");
                assert_eq!(
                    cfg.graphql.addr,
                    (Ipv4Addr::new(0, 0, 0, 0), 3000).into()
                );
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[graphql]
addr = "10.1.1.0:1234"
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.graphql.name, "unknown name");
                assert_eq!(cfg.graphql.location, "unknown location");
                assert_eq!(
                    cfg.graphql.addr,
                    (Ipv4Addr::new(10, 1, 1, 0), 1234).into()
                );
                assert_eq!(cfg.graphql.pref_host, None);
                assert_eq!(cfg.graphql.pref_port, 3000)
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[graphql]
pref_host = "www.google.com"
pref_port = 4000
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.graphql.name, "unknown name");
                assert_eq!(cfg.graphql.location, "unknown location");
                assert_eq!(
                    cfg.graphql.addr,
                    (Ipv4Addr::new(0, 0, 0, 0), 3000).into()
                );
                assert_eq!(
                    cfg.graphql.pref_host,
                    Some(String::from("www.google.com"))
                );
                assert_eq!(cfg.graphql.pref_port, 4000)
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[graphql]
pref_host = "www.google.com"
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.graphql.name, "unknown name");
                assert_eq!(cfg.graphql.location, "unknown location");
                assert_eq!(
                    cfg.graphql.addr,
                    (Ipv4Addr::new(0, 0, 0, 0), 3000).into()
                );
                assert_eq!(
                    cfg.graphql.pref_host,
                    Some(String::from("www.google.com"))
                );
                assert_eq!(cfg.graphql.pref_port, 3000)
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }
    }

    #[test]
    fn test_driver_section() {
        // Verify that the [[driver]] section needs an entry to be
        // defined..

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[driver]]
"#,
            )
            .is_err(),
            "TOML parser accepted empty [[driver]] section"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[driver]]
name = "none"
"#,
            )
            .is_err(),
            "TOML parser accepted [[driver]] section with missing prefix"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[driver]]
prefix = "null"
"#,
            )
            .is_err(),
            "TOML parser accepted [[driver]] section with missing name"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[driver]]
name = "none"
prefix = "null"
max_history = false
"#,
            )
            .is_err(),
            "TOML parser accepted [[driver]] section with bad max_history"
        );

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[[driver]]
name = "none"
prefix = "null"
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.driver.len(), 1);

                assert_eq!(cfg.driver[0].name, "none");
                assert_eq!(
                    cfg.driver[0].prefix,
                    "null".parse::<device::Path>().unwrap()
                );
                assert_eq!(cfg.driver[0].max_history, None);
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[[driver]]
name = "none"
prefix = "null"
max_history = 10000
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.driver.len(), 1);

                assert_eq!(cfg.driver[0].name, "none");
                assert_eq!(
                    cfg.driver[0].prefix,
                    "null".parse::<device::Path>().unwrap()
                );
                assert_eq!(cfg.driver[0].max_history, Some(10000));
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }
    }

    #[test]
    fn test_logic_section() {
        // Verify that the [[logic]] section needs an entry to be
        // defined..

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[logic]]"#
            )
            .is_err(),
            "TOML parser accepted empty [[logic]] section"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[logic]]
name = "none"
"#,
            )
            .is_err(),
            "TOML parser accepted [[logic]] section with missing 'exprs' and 'outputs'"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[logic]]
exprs = []
"#,
            )
            .is_err(),
            "TOML parser accepted [[logic]] section with missing 'name' and 'outputs'"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[logic]]
outputs = {}
"#,
            )
            .is_err(),
            "TOML parser accepted [[logic]] section with missing 'name' and 'exprs'"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[logic]]
name = "none"
exprs = []
"#,
            )
            .is_err(),
            "TOML parser accepted [[logic]] section with missing 'outputs'"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[logic]]
name = "none"
outputs = {}
"#,
            )
            .is_err(),
            "TOML parser accepted [[logic]] section with missing 'exprs'"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[logic]]
exprs = []
outputs = {}
"#,
            )
            .is_err(),
            "TOML parser accepted [[logic]] section with missing 'name'"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[logic]]
exprs = []
inputs = { bulb = "junk+name" }
outputs = {}
"#,
            )
            .is_err(),
            "TOML parser accepted [[logic]] section with bad device name in 'inputs'"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
latitude = -45.0
longitude = 45.0

[[logic]]
exprs = []
outputs = { bulb = "junk+name" }
"#,
            )
            .is_err(),
            "TOML parser accepted [[logic]] section with bad device name in 'outputs'"
        );

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[[logic]]
name = "none"
exprs = []
outputs = {}
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.logic.len(), 1);
                assert_eq!(cfg.logic[0].name, "none");
                assert!(cfg.logic[0].defs.is_empty());
                assert!(cfg.logic[0].exprs.is_empty());
                assert!(cfg.logic[0].inputs.is_empty());
                assert!(cfg.logic[0].outputs.is_empty());
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[[logic]]
name = "none"
exprs = []
outputs = { bulb = "room:bulb:enable" }
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.logic.len(), 1);
                assert_eq!(cfg.logic[0].name, "none");
                assert!(cfg.logic[0].defs.is_empty());
                assert!(cfg.logic[0].exprs.is_empty());
                assert!(cfg.logic[0].inputs.is_empty());
                assert_eq!(
                    cfg.logic[0].outputs.get("bulb"),
                    Some(&"room:bulb:enable".parse::<device::Name>().unwrap())
                );
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[[logic]]
name = "none"
exprs = []
inputs = { bulb = "room:bulb:enable" }
outputs = {}
"#,
        ) {
            Ok(cfg) => {
                assert_eq!(cfg.logic.len(), 1);
                assert_eq!(cfg.logic[0].name, "none");
                assert!(cfg.logic[0].defs.is_empty());
                assert!(cfg.logic[0].exprs.is_empty());
                assert!(cfg.logic[0].outputs.is_empty());
                assert_eq!(
                    cfg.logic[0].inputs.get("bulb"),
                    Some(&"room:bulb:enable".parse::<device::Name>().unwrap())
                );
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }
    }

    #[cfg(feature = "simple-backend")]
    #[test]
    fn test_simple_config() {}

    #[cfg(feature = "redis-backend")]
    #[test]
    fn test_redis_config() {
        // Verify a missing [backend] results in a properly defined
        // default.

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0
"#,
        ) {
            Ok(cfg) => {
                let def_cfg = Config::default();

                assert_eq!(
                    cfg.get_backend().get_addr(),
                    def_cfg.get_backend().get_addr()
                );
                assert_eq!(
                    cfg.get_backend().get_dbn(),
                    def_cfg.get_backend().get_dbn()
                );
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        // Verify the [backend] section can handle only one field at a
        // time.

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[backend]
addr = "192.168.1.1:6000"
"#,
        ) {
            Ok(cfg) => {
                let def_cfg = Config::default();

                assert_eq!(
                    cfg.get_backend().get_addr(),
                    "192.168.1.1:6000".parse().unwrap()
                );
                assert_eq!(
                    cfg.get_backend().get_dbn(),
                    def_cfg.get_backend().get_dbn()
                );
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
latitude = -45.0
longitude = 45.0

[backend]
dbn = 3
"#,
        ) {
            Ok(cfg) => {
                let def_cfg = Config::default();

                assert_eq!(
                    cfg.get_backend().get_addr(),
                    def_cfg.get_backend().get_addr()
                );
                assert_eq!(cfg.get_backend().get_dbn(), 3);
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }
    }
}
