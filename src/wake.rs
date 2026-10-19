//! Wake-on-LAN: a magic packet for a sleeping host (a Mac mini only answers it over Ethernet).
use anyhow::{bail, Result};
use std::net::UdpSocket;

/// `aa:bb:cc:dd:ee:ff` (or with `-`) as six bytes.
pub fn parse_mac(text: &str) -> Result<[u8; 6]> {
    let parts: Vec<&str> = text.trim().split([':', '-']).collect();
    if parts.len() != 6 {
        bail!("`{text}` is not a MAC address like aa:bb:cc:dd:ee:ff");
    }
    let mut mac = [0u8; 6];
    for (slot, part) in mac.iter_mut().zip(parts) {
        if part.len() != 2 {
            bail!("`{text}` is not a MAC address like aa:bb:cc:dd:ee:ff");
        }
        *slot = u8::from_str_radix(part, 16)
            .map_err(|_| anyhow::anyhow!("`{text}` is not a MAC address like aa:bb:cc:dd:ee:ff"))?;
    }
    Ok(mac)
}
/// Six 0xFF bytes followed by the MAC sixteen times.
pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut p = [0xFF; 102];
    for i in 0..16 {
        p[6 + i * 6..12 + i * 6].copy_from_slice(&mac);
    }
    p
}
/// Broadcast the magic packet to `broadcast` (default 255.255.255.255) on UDP port 9.
pub fn wake(mac: &str, broadcast: Option<&str>) -> Result<()> {
    let packet = magic_packet(parse_mac(mac)?);
    let target = format!("{}:9", broadcast.unwrap_or("255.255.255.255"));
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_broadcast(true)?;
    socket.send_to(&packet, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macs_parse_with_colons_or_dashes_and_reject_junk() {
        assert_eq!(
            parse_mac("AA:bb:0c:dd:ee:01").unwrap(),
            [0xaa, 0xbb, 0x0c, 0xdd, 0xee, 0x01]
        );
        assert!(parse_mac("aa-bb-cc-dd-ee-ff").is_ok());
        for bad in [
            "",
            "aa:bb:cc:dd:ee",
            "aa:bb:cc:dd:ee:gg",
            "aabb.ccdd.eeff",
            "a:b:c:d:e:f",
        ] {
            assert!(parse_mac(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn the_magic_packet_is_six_ff_then_the_mac_sixteen_times() {
        let mac = [1, 2, 3, 4, 5, 6];
        let p = magic_packet(mac);
        assert_eq!(&p[..6], &[0xFF; 6]);
        assert_eq!(p.len(), 102);
        assert!(p[6..].chunks(6).all(|c| c == mac));
    }
    #[test]
    fn a_packet_reaches_a_listener() {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let port = rx.local_addr().unwrap().port();
        let tx = UdpSocket::bind("0.0.0.0:0").unwrap();
        tx.send_to(&magic_packet([9; 6]), format!("127.0.0.1:{port}"))
            .unwrap();
        let mut buf = [0u8; 200];
        assert_eq!(rx.recv(&mut buf).unwrap(), 102);
    }
}
