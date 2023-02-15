use super::store;
use serde_derive::Deserialize;
use std::collections::HashMap;
use std::env;
use toml::{self, value};
use tracing::Level;

use drmem_api::{
    driver::DriverConfig,
    types::device::{Name, Path},
};

fn def_log_level() -> String {
    String::from("warn")
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "def_log_level")]
    log_level: String,
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
    pub prefix: Path,
    pub max_history: Option<usize>,
    pub cfg: Option<DriverConfig>,
}

#[derive(Deserialize)]
pub struct Logic {
    pub name: String,
    #[serde(default)]
    pub defs: HashMap<String, String>,
    pub exprs: Vec<String>,
    #[serde(default)]
    pub inputs: HashMap<String, Name>,
    pub outputs: HashMap<String, Name>,
}

fn from_cmdline(mut cfg: Config) -> (bool, Config) {
    use clap::{App, Arg};

    // Define the command line arguments.

    let matches = App::new("DrMem Mini Control System")
        .version("0.1")
        .about("A small, yet capable, control system.")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("FILE")
                .help("Specifies the configuration file")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("verbose")
                .short("v")
                .long("verbose")
                .multiple(true)
                .help("Sets verbosity of log; can be used more than once")
                .takes_value(false),
        )
        .arg(
            Arg::with_name("print_cfg")
                .long("print-config")
                .help("Displays the configuration and exits")
                .takes_value(false),
        )
        .get_matches();

    // The number of '-v' options determines the log level.

    match matches.occurrences_of("verbose") {
        0 => (),
        1 => cfg.log_level = String::from("info"),
        2 => cfg.log_level = String::from("debug"),
        _ => cfg.log_level = String::from("trace"),
    };

    // Return the config built from the command line and a flag
    // indicating the user wants the final configuration displayed.

    (matches.is_present("print_cfg"), cfg)
}

fn parse_config(path: &str, contents: &str) -> Option<Config> {
    match toml::from_str(contents) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            print!("ERROR: {},\n       ignoring {}\n", e, path);
            None
        }
    }
}

async fn from_file(path: &str) -> Option<Config> {
    use tokio::fs;

    if let Ok(contents) = fs::read(path).await {
        let contents = String::from_utf8_lossy(&contents);

        parse_config(path, &contents)
    } else {
        None
    }
}

async fn find_cfg() -> Config {
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
    Config::default()
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
    let cfg = find_cfg().await;
    let (print_cfg, cfg) = from_cmdline(cfg);

    if print_cfg {
        dump_config(&cfg);
        None
    } else {
        Some(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_config() {
        // Verify the defaults.

        match toml::from_str::<Config>("") {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::WARN),
            Err(e) => panic!("TOML parse error: {}", e),
        }

        // Verify the log_level can be set.

        match toml::from_str::<Config>("log_level = \"trace\"") {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::TRACE),
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>("log_level = \"debug\"") {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::DEBUG),
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>("log_level = \"info\"") {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::INFO),
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>("log_level = \"warn\"") {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::WARN),
            Err(e) => panic!("TOML parse error: {}", e),
        }
    }

    #[cfg(feature = "graphql")]
    #[test]
    fn test_graphql_config() {
        match toml::from_str::<Config>("") {
            Ok(cfg) => {
                assert_eq!(cfg.graphql.name, "unknown name");
                assert_eq!(cfg.graphql.location, "unknown location");
                assert_eq!(
                    cfg.graphql.addr,
                    (Ipv4Addr::new(0, 0, 0, 0), 3000).into()
                );
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
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
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
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
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }
    }

    #[test]
    fn test_driver_section() {
        // Verify that the [[driver]] section needs an entry to be
        // defined..

        assert!(
            toml::from_str::<Config>("[[driver]]").is_err(),
            "TOML parser accepted empty [[driver]] section"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
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
                    "null".parse::<Path>().unwrap()
                );
                assert_eq!(cfg.driver[0].max_history, None);
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
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
                    "null".parse::<Path>().unwrap()
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
            toml::from_str::<Config>("[[logic]]").is_err(),
            "TOML parser accepted empty [[logic]] section"
        );

        assert!(
            toml::from_str::<Config>(
                r#"
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
                    Some(&"room:bulb:enable".parse::<Name>().unwrap())
                );
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
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
                    Some(&"room:bulb:enable".parse::<Name>().unwrap())
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

        match toml::from_str::<Config>("") {
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
