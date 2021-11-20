// Copyright (c) 2020-2021, Richard M Neswold, Jr.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
// 1. Redistributions of source code must retain the above copyright
//    notice, this list of conditions and the following disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright
//    notice, this list of conditions and the following disclaimer in the
//    documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its
//    contributors may be used to endorse or promote products derived
//    from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tracing::Level;
use toml::value;
use serde_derive::{ Deserialize };

#[cfg(feature = "redis-backend")]
pub mod backend {
    use serde_derive::Deserialize;

    #[derive(Deserialize)]
    pub struct Config {
        pub addr: Option<String>,
        pub port: Option<u16>,
        pub dbn: Option<i64>,
    }

    impl<'a> Config {
        pub const fn new() -> Config {
            Config {
                addr: None,
                port: None,
                dbn: None,
            }
        }

        pub fn get_addr(&'a self) -> &'a str {
            if let Some(v) = &self.addr {
                v.as_str()
            } else {
                "127.0.0.1"
            }
        }

        pub fn get_port(&self) -> u16 {
            self.port.unwrap_or(6379)
        }

        #[cfg(debug_assertions)]
        pub fn get_dbn(&self) -> i64 {
            self.dbn.unwrap_or(1)
        }
        #[cfg(not(debug_assertions))]
        pub fn get_dbn(&self) -> i64 {
            self.dbn.unwrap_or(0)
        }
    }

    pub static DEF: Config = Config::new();
}

#[derive(Deserialize)]
pub struct Config {
    log_level: Option<String>,
    pub backend: Option<backend::Config>,
    pub driver: Vec<Driver>,
}

impl<'a> Config {
    pub fn get_log_level(&self) -> Level {
        if let Some(v) = &self.log_level {
            match v.as_str() {
                "info" => Level::INFO,
                "debug" => Level::DEBUG,
                "trace" => Level::TRACE,
                _ => Level::WARN,
            }
        } else {
            Level::WARN
        }
    }

    pub fn get_backend(&'a self) -> &'a backend::Config {
        if let Some(v) = &self.backend {
            v
        } else {
            &backend::DEF
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            log_level: None,
            backend: Some(backend::Config::new()),
            driver: vec![],
        }
    }
}

pub type DriverConfig = value::Table;

#[derive(Deserialize)]
pub struct Driver {
    pub name: String,
    pub prefix: Option<String>, // XXX: needs to be validated
    pub cfg: Option<value::Table>,
}

fn from_cmdline(mut cfg: Config) -> (bool, Config) {
    use clap::{App, Arg};

    // Define the command line arguments.

    let matches = App::new("DrMemory Mini Control System")
        .version("0.1")
        .author("Rich Neswold <rich.neswold@gmail.com>")
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
        1 => cfg.log_level = Some(String::from("info")),
        2 => cfg.log_level = Some(String::from("debug")),
        _ => cfg.log_level = Some(String::from("trace")),
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
    if let Some(cfg) = from_file("./drmem.toml").await {
        cfg
    } else {
        use std::env;

        if let Ok(home) = env::var("HOME") {
            if let Some(cfg) = from_file(&(home + "/.drmem.toml")).await {
                return cfg;
            }
        }
        if let Some(cfg) = from_file("/usr/local/etc/drmem.toml").await {
            cfg
        } else if let Some(cfg) = from_file("/usr/pkg/etc/drmem.toml").await {
            cfg
        } else if let Some(cfg) = from_file("/etc/drmem.toml").await {
            cfg
        } else {
            Config::default()
        }
    }
}

fn dump_config(cfg: &Config) -> () {
    print!("Configuration:\n");
    print!("    log level: {}\n\n", cfg.get_log_level());

    #[cfg(feature = "redis-backend")]
    {
        print!("Using REDIS for storage:\n");
        print!("    address: {}\n", &cfg.get_backend().get_addr());
        print!("    port: {}\n", cfg.get_backend().get_port());
        print!("    db #: {}\n\n", cfg.get_backend().get_dbn());
    }

    println!("Driver configuration:");
    if !cfg.driver.is_empty() {
        for ii in &cfg.driver {
            print!(
                "    name: {}, prefix: {}, cfg: {:?}",
                &ii.name,
                ii.prefix.as_ref().unwrap_or(&String::from("\"\"")),
                ii.cfg.as_ref().unwrap_or(&value::Table::new())
            )
        }
        print!("\n");
    } else {
        print!("    No drivers specified.\n");
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

    fn test_defaults() {
        // Verify that missing the [[driver]] section fails.

        if let Ok(_) = toml::from_str::<Config>("") {
            panic!("TOML parser accepted missing [[driver]] section")
        }

        // Verify that the [[driver]] section needs an entry to be
        // defined..

        if let Ok(_) = toml::from_str::<Config>(
            r#"
[[driver]]
"#,
        ) {
            panic!("TOML parser accepted missing [[driver]] section")
        }

        // Verify a missing [backend] results in a properly defined
        // default.

        match toml::from_str::<Config>(
            r#"
[[driver]]
name = "none"
"#,
        ) {
            Ok(cfg) => {
                let def_cfg = Config::default();

                #[cfg(feature = "redis-backend")]
                {
                    assert_eq!(
                        cfg.get_backend().get_addr(),
                        def_cfg.get_backend().get_addr()
                    );
                    assert_eq!(
                        cfg.get_backend().get_port(),
                        def_cfg.get_backend().get_port()
                    );
                    assert_eq!(
                        cfg.get_backend().get_dbn(),
                        def_cfg.get_backend().get_dbn()
                    );
                }

                assert_eq!(cfg.log_level, def_cfg.log_level);
                assert_eq!(cfg.driver.len(), 1)
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        // Verify the [backend] section can handle only one field at a
        // time.

        match toml::from_str::<Config>(
            r#"
[backend]
addr = "192.168.1.1"

[[driver]]
name = "none"
"#,
        ) {
            Ok(cfg) => {
                let def_cfg = Config::default();

                #[cfg(feature = "redis-backend")]
                {
                    assert_eq!(cfg.get_backend().get_addr(), "192.168.1.1");
                    assert_eq!(
                        cfg.get_backend().get_port(),
                        def_cfg.get_backend().get_port()
                    );
                    assert_eq!(
                        cfg.get_backend().get_dbn(),
                        def_cfg.get_backend().get_dbn()
                    );
                }
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
[backend]
port = 7000

[[driver]]
name = "none"
"#,
        ) {
            Ok(cfg) => {
                let def_cfg = Config::default();

                #[cfg(feature = "redis-backend")]
                {
                    assert_eq!(
                        cfg.get_backend().get_addr(),
                        def_cfg.get_backend().get_addr()
                    );
                    assert_eq!(cfg.get_backend().get_port(), 7000);
                    assert_eq!(
                        cfg.get_backend().get_dbn(),
                        def_cfg.get_backend().get_dbn()
                    );
                }
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        match toml::from_str::<Config>(
            r#"
[backend]
dbn = 3

[[driver]]
name = "none"
"#,
        ) {
            Ok(cfg) => {
                let def_cfg = Config::default();

                #[cfg(feature = "redis-backend")]
                {
                    assert_eq!(
                        cfg.get_backend().get_addr(),
                        def_cfg.get_backend().get_addr()
                    );
                    assert_eq!(
                        cfg.get_backend().get_port(),
                        def_cfg.get_backend().get_port()
                    );
                    assert_eq!(cfg.get_backend().get_dbn(), 3);
                }
            }
            Err(e) => panic!("TOML parse error: {}", e),
        }

        // Verify the log_level can be set.

        match toml::from_str::<Config>(
            r#"
log_level = "trace"
[[driver]]
name = "none"
"#,
        ) {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::TRACE),
            Err(e) => panic!("TOML parse error: {}", e),
        }
        match toml::from_str::<Config>(
            r#"
log_level = "debug"
[[driver]]
name = "none"
"#,
        ) {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::DEBUG),
            Err(e) => panic!("TOML parse error: {}", e),
        }
        match toml::from_str::<Config>(
            r#"
log_level = "info"
[[driver]]
name = "none"
"#,
        ) {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::INFO),
            Err(e) => panic!("TOML parse error: {}", e),
        }
        match toml::from_str::<Config>(
            r#"
log_level = "warn"
[[driver]]
name = "none"
"#,
        ) {
            Ok(cfg) => assert_eq!(cfg.get_log_level(), Level::WARN),
            Err(e) => panic!("TOML parse error: {}", e),
        }
    }

    #[tokio::test]
    async fn test_config() {
        test_defaults()
    }
}
