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

use tracing::{ warn, info, trace, Level };
use toml::value;
use serde_derive::{ Serialize, Deserialize };

#[cfg(feature = "redis-backend")]
#[derive(Serialize,Deserialize)]
pub struct RedisConfig {
    pub addr: String,
    pub port: u16,
    pub dbn: i64
}

#[cfg(feature = "redis-backend")]
impl Default for RedisConfig {
    fn default() -> Self {
	RedisConfig {
	    addr: String::from("127.0.0.1"),
	    port: 6379,
	    dbn: 0
	}
    }
}

#[derive(Serialize,Deserialize)]
pub struct Config {
    log_level: String,
    #[cfg(feature = "redis-backend")]
    pub redis: RedisConfig,
    pub driver: Vec<Driver>
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
	    log_level: String::from("warn"),
	    #[cfg(feature = "redis-backend")]
	    redis: RedisConfig::default(),
	    driver: vec![]
	}
    }
}

#[derive(Serialize,Deserialize)]
pub struct Driver {
    pub name: String,
    pub prefix: String,		// XXX: needs to be validated
    pub cfg: value::Table
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

    // The number of '-v' options determines the log level.

    match matches.occurrences_of("verbose") {
        0 => (),
        1 => cfg.log_level = String::from("info"),
        2 => cfg.log_level = String::from("debug"),
	_ => cfg.log_level = String::from("trace")
    };

    // Return the config built from the command line and a flag
    // indicating the user wants the final configuration displayed.

    (matches.is_present("print_cfg"), cfg)
}

async fn from_file(path: &str) -> Option<Config> {
    use tokio::fs;

    trace!("looking for file `{}`", path);
    if let Ok(contents) = fs::read(path).await {
	let contents = String::from_utf8_lossy(&contents);

	match toml::from_str(&contents) {
	    Ok(cfg) => {
		info!("reading config from `{}`", path);
		return Some(cfg)
	    }
	    Err(e) => {
		warn!("error parsing `{}` : '{:?}' ... ignoring", path, e);
	    }
	}
    } else {
	trace!("unable to read `{}`", path)
    }
    None
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

#[tracing::instrument(name = "loading config")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_config() {
	#[cfg(feature = "redis-backend")]
	let cfg = r#"
log_level = "info"

[redis]
addr = '127.0.0.1'
port = 100
dbn = 0

# This section defines the drivers that get loaded.

[[driver]]
name = 'hue'
prefix = 'hue'
cfg = { ip = '127.0.0.1', key = 'blah' }
"#;

	#[cfg(not(feature = "redis-backend"))]
	let cfg = r#"
log_level = "info"

# This section defines the drivers that get loaded.

[[driver]]
name = 'hue'
prefix = 'hue'
cfg = { ip = '127.0.0.1', key = 'blah' }
"#;

	if let Err(e) = toml::from_str::<Config>(cfg) {
	    panic!("couldn't parse: {}", e)
	}
    }
}
