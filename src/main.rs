use chrono::Local;
use reqwest::blocking::Client;
use serde_json::Value;
use std::{
    collections::HashMap,
    env, fs,
    io::{self},
    path::Path,
    process::Command,
    os::unix::fs::PermissionsExt
};
use inline_colorization::*;

const DEFAULT_CONFIG_FILES: [&str; 4] = [
    "./opnsense-bind9.conf",
    "~/.opnsense-bind9.conf",
    "/usr/local/etc/opnsense-bind9.conf",
    "/etc/opnsense-bind9.conf",
];
type Config = HashMap<String, String>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let config_file = if let Some(index) = args.iter().position(|arg| arg == "--conf") {
        args.get(index + 1).cloned()
    } else {
        None
    };

    let config = read_config(config_file)?;

    let default_rndc_cmd = "/usr/sbin/rndc".to_string();
    let rndc_cmd = config.get("RNDC_CMD").unwrap_or(&default_rndc_cmd);
    let api_url = config.get("API_URL").ok_or("Missing API_URL in config")?;
    let api_user = config.get("API_USER").ok_or("Missing API_USER in config")?;
    let api_pass = config.get("API_PASS").ok_or("Missing API_PASS in config")?;
    let default_base_dir = "/etc/bind".to_string();
    let base_dir = config.get("BASE_DIR").unwrap_or(&default_base_dir);

    let zone_name = config
        .get("ZONE_NAME")
        .ok_or("Missing ZONE_NAME in config")?;

    let main_conf = format!(
        "{}/{}",
        base_dir,
        config
            .get("MAIN_CONF")
            .ok_or("Missing MAIN_CONF in config")?
    );

    let host_file = format!(
        "{}/{}",
        base_dir,
        config
            .get("HOST_FILE")
            .ok_or("Missing HOST_FILE in config")?
    );

    let subnet_prefix = config
        .get("SUBNET_PREFIX")
        .ok_or("Missing SUBNET_PREFIX in config")?
        .trim_end_matches('.');

    let reversed_subnet: String = subnet_prefix.split('.').rev().collect::<Vec<_>>().join(".");
    let ptr_zone_name = format!("{}.in-addr.arpa", reversed_subnet);
    let ptr_conf = format!("{}/db.{}", base_dir, ptr_zone_name);

    let ptr_file = format!(
        "{}/{}",
        base_dir,
        config.get("PTR_FILE").ok_or("Missing PTR_FILE in config")?
    );

    let ignored_hosts: Vec<String> = config
        .get("IGNORED_HOSTS")
        .map(|v| v.split(',').map(|s| s.trim().to_lowercase()).collect())
        .unwrap_or_default();

    let ignored_ptr: Vec<String> = config
        .get("IGNORED_PTR")
        .map(|v| v.split(',').map(|s| s.trim().to_lowercase()).collect())
        .unwrap_or_default();

    if !Path::new(base_dir).exists() {
        eprintln!("{color_red}[ERROR]{color_reset} Base directory {} does not exist.", base_dir);
        return Err("Base directory does not exist".into());
    }

    if !Path::new(&main_conf).exists() || fs::metadata(&main_conf)?.permissions().readonly() {
        eprintln!(
            "{color_red}[ERROR]{color_reset} Main config file {} does not exist or is not writable.",
            main_conf
        );
        return Err("Main config file is missing or not writeable".into());
    }

    if !Path::new(&ptr_conf).exists() || fs::metadata(&ptr_conf)?.permissions().readonly() {
        eprintln!(
            "{color_red}[ERROR]{color_reset} PTR-zone config file {} does not exist or is not writable.",
            ptr_conf
        );
        return Err("PTR-zone config file is missing or not writeable".into());
    }

    if !Path::new(rndc_cmd).exists() || !Path::new(rndc_cmd).metadata()?.permissions().mode() & 0o111 != 0
    {
        eprintln!(
            "{color_red}[ERROR]{color_reset} rndc-command {} does not exist or is not executable.",
            rndc_cmd
        );
        return Err("rndc-command is missing or not executable".into());
    }

    let client = Client::new();

    let response = client
        .get(api_url)
        .basic_auth(api_user, Some(api_pass))
        .send()?;

    if !response.status().is_success() {
        eprintln!("{color_red}[ERROR]{color_reset} Failed to fetch DHCP leases: {}", response.status());
        return Err("Failed to fetch DHCP leases".into());
    }

    let json: Value = response.json()?;

    let leases = json["rows"]
        .as_array()
        .ok_or("Invalid API response format")?;

    if leases.is_empty() {
        eprintln!("{color_red}[ERROR]{color_reset} No hosts received from API, exiting.");
        return Err("No hosts received".into());
    }

    let mut host_records = String::new();
    let mut ptr_records = String::new();

    for lease in leases {
        if let (Some(hostname), Some(ip)) = (lease["hostname"].as_str(), lease["address"].as_str())
        {
            if ip.starts_with(subnet_prefix) {
                if !ignored_hosts.contains(&hostname.to_lowercase())
                    && !ignored_hosts.contains(&ip.to_string())
                {
                    host_records.push_str(&format!("{:<24}\tIN\tA\t{}\n", hostname, ip));
                }
                if !ignored_ptr.contains(&hostname.to_lowercase())
                    && !ignored_ptr.contains(&ip.to_string())
                {
                    if let Some(last_octet) = ip.rsplit('.').next() {
                        ptr_records.push_str(&format!(
                            "{}\tIN\tPTR\t{}.{}.\n",
                            last_octet, hostname, zone_name
                        ));
                    }
                }
            }
        }
    }

    fs::write(&host_file, host_records)?;
    println!("{color_green}[OK]{color_reset}    Updated {}", ptr_file);
    update_serial_number(&main_conf)?;

    fs::write(&ptr_file, ptr_records)?;
    println!("{color_green}[OK]{color_reset}    Updated {}", ptr_file);
    update_serial_number(&ptr_conf)?;

    if let Err(e) = reload_zone(rndc_cmd, &zone_name) {
        eprintln!("{}", e);
    }

    if let Err(e) = reload_zone(rndc_cmd, &ptr_zone_name){
        eprintln!("{}", e);
    }

    Ok(())
}

fn reload_zone(rndc_cmd: &str, zone: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new(rndc_cmd)
        .arg("reload")
        .arg(zone)
        .status()?;

    if status.success() {
        println!("{color_green}[OK]{color_reset}     Reloaded zone {}", zone);
        return Ok(());
    }
    return Err(format!("{color_red}[ERROR]{color_reset} Failed to reload zone: {}", zone).into());
}

fn read_config(config_file: Option<String>) -> Result<Config, Box<dyn std::error::Error>> {
    let config_paths = if let Some(file) = config_file {
        vec![file]
    } else {
        DEFAULT_CONFIG_FILES
            .iter()
            .map(|&s| {
                s.to_string()
                    .replace("~", &env::var("HOME").unwrap_or_default())
                    .to_string()
            })
            .collect()
    };

    for path in config_paths {
        if let Ok(content) = fs::read_to_string(&path) {
            println!("{color_yellow}[INFO]{color_reset}  Using config file {}", path);
            let config = content
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
                .filter_map(|line| {
                    let parts: Vec<&str> = line.splitn(2, '=').map(str::trim).collect();
                    if parts.len() == 2 {
                        Some((parts[0].to_string(), parts[1].to_string()))
                    } else {
                        None
                    }
                })
                .collect();
            return Ok(config);
        }
    }

    Err("No valid config file found".into())
}

fn update_serial_number(zone_file: &str) -> io::Result<()> {
    let contents = fs::read_to_string(zone_file)?;
    let mut lines: Vec<String> = contents.lines().map(|s| s.to_string()).collect();
    let today = Local::now().format("%Y%m%d").to_string();
    let mut new_serial = format!("{}00", today);

    for line in &mut lines {
        if line.contains(";") && line.contains("serial") {
            if let Some(num) = line.split_whitespace().next() {
                if num.starts_with(&today) {
                    if let Ok(serial) = num.parse::<u32>() {
                        new_serial = format!("{}", serial + 1);
                    }
                }
            }
            *line = format!("\t{}\t; serial", new_serial);
            break;
        }
    }

    fs::write(zone_file, format!("{}\n", lines.join("\n")))?;
    println!("{color_green}[OK]{color_reset}    Updated serial number in: {}", zone_file);

    Ok(())
}
