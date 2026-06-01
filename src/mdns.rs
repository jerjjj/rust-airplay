use std::collections::HashMap;

use anyhow::Result;
use crate::config::Config;

pub struct MdnsHandle {
    daemon: mdns_sd::ServiceDaemon,
    airplay_fullname: String,
    airplay_bds_fullname: String,
    raop_fullname: String,
}

impl MdnsHandle {
    pub async fn unregister(self) -> Result<()> {
        self.daemon.unregister(&self.airplay_fullname)?;
        self.daemon.unregister(&self.airplay_bds_fullname)?;
        self.daemon.unregister(&self.raop_fullname)?;
        Ok(())
    }
}

const MDNS_HOSTNAME: &str = "rust-airplay.local.";
const AIRPLAY_TYPE: &str = "_airplay._tcp.local.";
const AIRPLAY_BDS_TYPE: &str = "_airplay-bds._tcp.local.";
const AIRTUNES_TYPE: &str = "_raop._tcp.local.";

pub async fn register(config: &Config) -> Result<MdnsHandle> {
    let daemon = mdns_sd::ServiceDaemon::new()?;

    let airplay_fullname = register_airplay(&daemon, config)?;
    let airplay_bds_fullname = register_airplay_bds(&daemon, config)?;
    let raop_fullname = register_raop(&daemon, config)?;

    Ok(MdnsHandle { daemon, airplay_fullname, airplay_bds_fullname, raop_fullname })
}

fn register_airplay_bds(daemon: &mdns_sd::ServiceDaemon, config: &Config) -> Result<String> {
    let port = config.airtunes_port;
    let mut props: HashMap<String, String> = HashMap::new();
    props.insert("deviceid".to_string(), config.mac_address.clone());

    let service = mdns_sd::ServiceInfo::new(
        AIRPLAY_BDS_TYPE, &config.server_name, MDNS_HOSTNAME, "", port, props,
    )?.enable_addr_auto();

    let fullname = service.get_fullname().to_string();
    daemon.register(service)?;
    tracing::info!("AirPlay BDS registered: {}", fullname);
    Ok(fullname)
}

fn register_airplay(daemon: &mdns_sd::ServiceDaemon, config: &Config) -> Result<String> {
    let port = config.airtunes_port;

    let mut props: HashMap<String, String> = HashMap::new();
    props.insert("deviceid".to_string(), config.mac_address.clone());
    props.insert("features".to_string(), "0x5A7FFFF7,0x1E".to_string());
    props.insert("srcvers".to_string(), "220.68".to_string());
    props.insert("flags".to_string(), "0x0".to_string());
    props.insert("vv".to_string(), "2".to_string());
    props.insert("model".to_string(), "AppleTV3,2C".to_string());
    props.insert("rhd".to_string(), "5.6.0.0".to_string());
    props.insert("pw".to_string(), "false".to_string());
    props.insert("pk".to_string(), "f3769a660475d27b4f6040381d784645e13e21c53e6d2da6a8c3d757086fc336".to_string());
    props.insert("rmodel".to_string(), "PC1.0".to_string());
    props.insert("rrv".to_string(), "1.01".to_string());
    props.insert("rsv".to_string(), "1.00".to_string());
    props.insert("pcversion".to_string(), "1715".to_string());
    props.insert("pi".to_string(), "b08f5a79-db29-4384-b456-a4784d9e6055".to_string());

    let service = mdns_sd::ServiceInfo::new(
        AIRPLAY_TYPE, &config.server_name, MDNS_HOSTNAME, "", port, props,
    )?.enable_addr_auto();

    let fullname = service.get_fullname().to_string();
    daemon.register(service)?;
    tracing::warn!("AirPlay registered: {} on port {}", fullname, port);
    Ok(fullname)
}

fn register_raop(daemon: &mdns_sd::ServiceDaemon, config: &Config) -> Result<String> {
    let mac_no_colon = config.mac_address.replace(':', "");
    let instance_name = format!("{}@{}", mac_no_colon, config.server_name);
    let port = config.airtunes_port;

    let mut props: HashMap<String, String> = HashMap::new();
    props.insert("ch".to_string(), "2".to_string());
    props.insert("cn".to_string(), "1,3".to_string());
    props.insert("da".to_string(), "true".to_string());
    props.insert("et".to_string(), "0,3,5".to_string());
    props.insert("ek".to_string(), "1".to_string());
    props.insert("ft".to_string(), "0x5A7FFFF7,0x1E".to_string());
    props.insert("am".to_string(), "AppleTV3,2C".to_string());
    props.insert("md".to_string(), "0,1,2".to_string());
    props.insert("rhd".to_string(), "5.6.0.0".to_string());
    props.insert("sr".to_string(), "44100".to_string());
    props.insert("ss".to_string(), "16".to_string());
    props.insert("sv".to_string(), "false".to_string());
    props.insert("sm".to_string(), "false".to_string());
    props.insert("tp".to_string(), "UDP".to_string());
    props.insert("txtvers".to_string(), "1".to_string());
    props.insert("sf".to_string(), "0x44".to_string());
    props.insert("vs".to_string(), "220.68".to_string());
    props.insert("vn".to_string(), "65537".to_string());
    props.insert("vv".to_string(), "2".to_string());
    props.insert("pk".to_string(), "f3769a660475d27b4f6040381d784645e13e21c53e6d2da6a8c3d757086fc336".to_string());

    let service = mdns_sd::ServiceInfo::new(
        AIRTUNES_TYPE, instance_name.as_str(), MDNS_HOSTNAME, "", port, props,
    )?.enable_addr_auto();

    let fullname = service.get_fullname().to_string();
    daemon.register(service)?;
    tracing::info!("AirTunes registered: {} on port {}", fullname, port);
    Ok(fullname)
}
