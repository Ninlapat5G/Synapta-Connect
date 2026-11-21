//! รายการ network interface ของเครื่อง (LAN / Wi-Fi / VPN / virtual) ที่ใช้ได้จริง

use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr};

#[derive(Serialize, Clone)]
pub struct NetIf {
    pub name: String,
    pub ip: IpAddr,
    /// physical = LAN/Wi-Fi จริง, vpn = Tailscale/WireGuard/ZeroTier ฯลฯ, virtual = Hyper-V/Docker/VM
    pub kind: &'static str,
    #[serde(skip)]
    pub broadcast: Option<Ipv4Addr>,
}

/// เดาชนิดจากชื่อ adapter — OS ไม่บอกตรง ๆ จึงเป็นการเดาเท่านั้น (ใช้แสดงผล ไม่ใช้ตัดสินสิทธิ์)
fn classify(name: &str) -> &'static str {
    let n = name.to_lowercase();
    const VPN: [&str; 7] = [
        "tailscale",
        "wireguard",
        "zerotier",
        "radmin",
        "hamachi",
        "openvpn",
        "vpn",
    ];
    const VIRTUAL: [&str; 10] = [
        "vethernet",
        "docker",
        "veth",
        "br-",
        "vmnet",
        "virtualbox",
        "vmware",
        "teredo",
        "isatap",
        "6to4",
    ];
    // wg0 / tun0 / tap0 / utun3: เทียบแค่ต้นชื่อ (ไม่งั้น "Tunneling" ของ Teredo จะกลายเป็น VPN)
    let vpn_prefix = ["wg", "tun", "tap", "utun"].iter().any(|p| n.starts_with(p));
    if VIRTUAL.iter().any(|k| n.contains(k)) {
        "virtual"
    } else if vpn_prefix || VPN.iter().any(|k| n.contains(k)) {
        "vpn"
    } else {
        "physical"
    }
}

/// ที่อยู่ IPv4 ของ interface ที่ขึ้นอยู่ ไม่ใช่ loopback และไม่ใช่ link-local (เชื่อมต่อจากเครื่องอื่นไม่ได้)
/// เฉพาะ IPv4 เพราะ agent bind แค่ 0.0.0.0 — ไม่แสดง IPv6 ที่ต่อเข้ามาไม่ได้จริง (ยังไม่รองรับ IPv6)
pub fn list() -> Vec<NetIf> {
    let Ok(all) = if_addrs::get_if_addrs() else {
        return vec![];
    };
    all.into_iter()
        .filter(|i| i.is_oper_up() && !i.is_loopback())
        .filter(|i| matches!(i.ip(), IpAddr::V4(ip) if !ip.is_link_local()))
        .map(|i| NetIf {
            kind: classify(&i.name),
            broadcast: match &i.addr {
                if_addrs::IfAddr::V4(v4) => v4.broadcast,
                _ => None,
            },
            ip: i.ip(),
            name: i.name,
        })
        .collect()
}

pub fn local_ipv4_mask(ip: Ipv4Addr) -> Option<Ipv4Addr> {
    if_addrs::get_if_addrs().ok()?.into_iter().find_map(|interface| {
        if !interface.is_oper_up() || classify(&interface.name) != "physical" {
            return None;
        }
        match interface.addr {
            if_addrs::IfAddr::V4(address) if address.ip == ip => Some(address.netmask),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::classify;

    #[test]
    fn classifies_common_adapter_names() {
        assert_eq!(classify("Tailscale"), "vpn");
        assert_eq!(classify("wg0"), "vpn");
        assert_eq!(classify("vEthernet (WSL)"), "virtual");
        assert_eq!(classify("Wi-Fi"), "physical");
        assert_eq!(classify("Ethernet"), "physical");
        assert_eq!(classify("Radmin VPN"), "vpn");
        assert_eq!(classify("Teredo Tunneling Pseudo-Interface"), "virtual");
    }
}
