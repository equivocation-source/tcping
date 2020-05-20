use std::net::{TcpStream, SocketAddr, ToSocketAddrs};
use std::env;
use std::process;
use std::collections::HashMap;
use std::time::Instant;
use std::thread;

struct ResultCollection {
    iterations: u32,
    successes: u32,
    millis_min: f64,
    millis_max: f64,
    millis_squared: f64,
    millis_added: f64,
}

impl ResultCollection {
    fn new() -> ResultCollection {
        let millis_min = std::f64::MAX;
        let millis_max = std::f64::MIN;
        ResultCollection { iterations: 0, successes: 0, millis_min, millis_max, millis_squared: 0.0, millis_added: 0.0 }
    }

    fn add_interval(&mut self, successful: bool, millis: f64) {
        self.iterations = self.iterations + 1;
        if successful {
            self.successes = self.successes + 1;
            self.millis_added = self.millis_added + millis;
            self.millis_squared = self.millis_squared + (millis * millis);
            if millis < self.millis_min {
                self.millis_min = millis;
            }
            if millis > self.millis_max {
                self.millis_max = millis;
            }
        }
    }

    fn get_avg(&self) -> f64 {
        match self.successes {
            0 => 0.0,
            _ => self.millis_added / (self.successes as f64),
        }
    }

    fn get_std_dev(&self) -> f64 {
        if self.successes == 0 {
            0.0
        } else {
            let avg = self.millis_added / (self.successes as f64);
            let variance = (self.millis_squared + (self.successes as f64 * avg * avg) - (2.0 * self.millis_added * avg)) / (self.successes as f64);
            variance.sqrt()
        }
    }

    fn get_min(&self) -> f64 {
        match self.successes {
            0 => 0.0,
            _ => self.millis_min,
        }
    }

    fn get_max(&self) -> f64 {
        match self.successes {
            0 => 0.0,
            _ => self.millis_max,
        }
    }
}

struct ProgParameters {
    target_host: String,
    target_port: String,
    interval_count: u32,
    connection_timeout: std::time::Duration,
    wait_interval: std::time::Duration,
    bare_socket: SocketAddr,
}

#[derive(Hash, Eq, PartialEq)]
enum CmdLineOpts {
    AppName,
    HostName,
    PortVal,
    Intervals,
    TimeOut,
    Wait,
    Unset,
}

impl ProgParameters {
    fn new(args: &[String]) -> Result<ProgParameters, &'static str> {
        let args_iter = args.iter();
        let mut option_map = HashMap::new();
        let mut cmd_line_opt = CmdLineOpts::AppName;

        for arg in args_iter {
            match cmd_line_opt {
                //Pass first argument (app name)
                CmdLineOpts::AppName => cmd_line_opt = CmdLineOpts::Unset,
                //target unset
                CmdLineOpts::Unset => {
                    cmd_line_opt = match arg.as_str() {
                        "-h" => CmdLineOpts::HostName,
                        "-p" => CmdLineOpts::PortVal,
                        "-i" => CmdLineOpts::Intervals,
                        "-t" => CmdLineOpts::TimeOut,
                        "-w" => CmdLineOpts::Wait,
                        _ => return Err("Invalid Parameters"),
                    }
                },
                //target set, add to hashmap
                _ => {
                    option_map.insert(cmd_line_opt, arg);
                    cmd_line_opt = CmdLineOpts::Unset;
                },
            }
        }
        //deal with dangling option
        if CmdLineOpts::Unset != cmd_line_opt {
            return Err("Invalid Parameters");
        }
        
        let interval_count = match option_map.get(&CmdLineOpts::Intervals) {
            Some(val) => {
                match val.parse::<u32>() {
                    Ok(interval) => interval,
                    Err(_) => return Err("Invalid Interval"),
                }
            }
            None => return Err("Missing Intervals argument"),
        };

        if interval_count < 1 {
            return Err("Need at least 1 Interval");
        }

        let wait_interval = match option_map.get(&CmdLineOpts::Wait) {
            Some(val) => {
                match val.parse::<u64>() {
                    Ok(wait) => if wait < 1 { std::time::Duration::from_secs(1) } else { std::time::Duration::from_secs(wait) },
                    Err(_) => return Err("Invalid Wait"),
                }
            }
            None => std::time::Duration::from_secs(1),
        };
        let connection_timeout = match option_map.get(&CmdLineOpts::TimeOut) {
            Some(val) => {
                match val.parse::<u64>() {
                    Ok(timeout) => if timeout < 1 { std::time::Duration::from_secs(1) } else { std::time::Duration::from_secs(timeout) },
                    Err(_) => return Err("Invalid Timeout"),
                }
            }
            None => std::time::Duration::from_secs(5),
        };

        let target_host = match option_map.get(&CmdLineOpts::HostName) {
            Some(val) => val.to_string(),
            None => return Err("Host required!"),
        };

        let target_port = match option_map.get(&CmdLineOpts::PortVal) {
            Some(val) => val.to_string(),
            None => return Err("Port required!"),
        };

        let mut socket_iter = match format!("{}:{}",target_host, target_port).to_socket_addrs() {
            Ok(iter) => iter,
            Err(_) => return Err("Invalid host/port"),
        };

        let bare_socket = match socket_iter.next() {
            Some(socket) => socket,
            None => return Err("Unresolvable host/port"),
        };

        Ok(ProgParameters {interval_count, wait_interval, connection_timeout, target_host, target_port, bare_socket})
    }

    fn get_usage() -> &'static str {
        return "tcping -h -p -i -t -w\n\n \
        \t-h\t(required) Host name, ipv4, or ipv6 address\n \
        \t-p\t(required) Port (1-65535)\n \
        \t-i\t(required) Intervals.  Number of tests to run before exit\n \
        \t-t\tConnection Timeout. Wait before failing connection attempt (Default: OS Defined)\n \
        \t-w\tWait Interval. Wait between intervals in seconds (Default: 1)\n"
    }
}

fn run_connection_tests(result_col: &mut ResultCollection, prog_params: &ProgParameters) {
    loop {
        let now = Instant::now();
        match TcpStream::connect_timeout(&prog_params.bare_socket, prog_params.connection_timeout) {
            Ok(stream) => {
                let millis = (now.elapsed().as_micros() as f64) / 1000.0; //millis as a fraction
                println!("Connected {}:{} - {:.3}ms", prog_params.target_host, prog_params.target_port, millis);
                result_col.add_interval(true, millis);
                stream.shutdown(std::net::Shutdown::Both).unwrap();
            },
            Err(error) => {
                println!("Failed {}:{} - {}", prog_params.target_host, prog_params.target_port, error);
                result_col.add_interval(false, 0.0);
            },
        }
        if result_col.iterations == prog_params.interval_count {
            break;
        } else {
            thread::sleep(prog_params.wait_interval);
        }
    }
}

fn display_summary(result_col: &ResultCollection, prog_params: &ProgParameters) {
    println!("\nTCPING to {}:{}\n{} successes / {} attempts, min/max/avg/dev {:.3}/{:.3}/{:.3}/{:.3}",
             prog_params.target_host,
             prog_params.target_port,
             result_col.successes,
             result_col.iterations,
             result_col.get_min(),
             result_col.get_max(),
             result_col.get_avg(),
             result_col.get_std_dev());
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let prog_params = ProgParameters::new(&args).unwrap_or_else(|err| {
        println!("\nERROR: {}\n\nUsage: {}", err, ProgParameters::get_usage());
        process::exit(1);
    });
    let mut result_collection = ResultCollection::new();
    run_connection_tests(&mut result_collection, &prog_params);
    display_summary(&result_collection, &prog_params);
}
