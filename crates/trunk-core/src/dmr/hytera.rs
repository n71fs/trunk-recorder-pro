//! Hytera short-data applications. See docs/guides/dmr-gps.md for sources,
//! capture-based compressed GPS layout, and deliberately limited flag support.

use super::data::Packet;
use crate::position::Position;
use crate::trunk::message::{Message, MessageType};

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Decode only packets already checked by the data assembler. Unknown
/// applications remain visible as raw data, without creating voice grants.
pub fn packet(p: &Packet) -> Message {
    let mut m = Message {
        kind: MessageType::Status,
        source: p.source() as i64,
        meta: format!("DMR short data from {} to {}: {}", p.source(), p.destination(), hex(&p.payload)),
        ..Default::default()
    };
    let b = &p.payload;
    if b.starts_with(&[1, 0, 1, 0xd0, 3, 0]) {
        m.kind = MessageType::Location;
        m.meta = format!("Hytera compressed GPS from {}: {}", p.source(), hex(b));
        if let Some(pos) = compressed_gps(p) {
            m.meta = format!(
                "Hytera compressed GPS from {}: {:.6}, {:.6} (experimental N/W profile; UTC {:02}:{:02}:{:02}, {:02}-{:02}-{:02}; raw {})",
                pos.unit,
                pos.lat,
                pos.lon,
                b[9],
                b[10],
                b[11],
                b[14],
                b[13],
                b[12],
                hex(b)
            );
            m.position = Some(pos);
        }
    } else if b.starts_with(&[2, 0, 0x11, 0]) {
        // Short data has a two-octet service prefix, then an HDAP frame.
        let h = &b[2..];
        if let Some((op, data)) = hdap(h) {
            if let Some(ip) = data.get(..4) {
                let address = format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                let detail = match (op, data.len()) {
                    (3, 4) => {
                        m.kind = MessageType::Registration;
                        "registration request".into()
                    }
                    (1, 4) => {
                        m.kind = MessageType::Deregistration;
                        "offline notification".into()
                    }
                    (2, 4) => "status check".into(),
                    (0x80, 9) => {
                        m.kind = MessageType::Acknowledge;
                        format!("registration answer result {} renewal {} s", data[4], u32::from_be_bytes(data[5..9].try_into().unwrap()))
                    }
                    (0x82, 5) => format!("status answer state {}", data[4]),
                    _ => format!("opcode 0x{op:04x}"),
                };
                // The answering dispatcher is the sender, not the registered
                // radio. Keep both the air source and the RRS IP explicitly.
                m.meta = format!("Hytera RRS {detail} for IP {address}, from {} to {}", p.source(), p.destination());
            }
        }
    }
    m
}

/// RRS uses big-endian opcode/length; the checksum excludes service ID.
fn hdap(b: &[u8]) -> Option<(u16, &[u8])> {
    if b.len() < 7 {
        return None;
    }
    let n = u16::from_be_bytes([b[3], b[4]]) as usize;
    if b.len() != n + 7 || b[n + 6] != 3 {
        return None;
    }
    let sum = b[1..n + 5].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    if (!sum).wrapping_add(0x33) != b[n + 5] {
        return None;
    }
    Some((u16::from_be_bytes([b[1], b[2]]), &b[5..5 + n]))
}

/// Observed 25-byte compressed LP report, plus its two-byte short-data
/// prefix. Only the observed 0x8/0x8 N/W flag profile is supported. Other
/// hemispheres/fix flags remain raw; do not guess their meanings.
pub fn compressed_gps(p: &Packet) -> Option<Position> {
    let b = &p.payload;
    if p.source() == 0 || b.len() != 27 || !b.starts_with(&[1, 0, 1, 0xd0, 3, 0]) {
        return None;
    }
    // Binary UTC hh mm ss dd MM yy. Reject absent or malformed fixes.
    let (hour, min, sec, day, month, year) = (b[9], b[10], b[11], b[12], b[13], b[14]);
    if hour > 23 || min > 59 || sec > 60 || month == 0 || month > 12 || year > 99 {
        return None;
    }
    let days = match month {
        2 => {
            if year % 4 == 0 {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > days {
        return None;
    }
    let lat = u32::from_le_bytes(b[15..19].try_into().unwrap());
    let lon = u32::from_le_bytes(b[19..23].try_into().unwrap());
    if lat >> 28 != 8 || lon >> 28 != 8 {
        return None;
    }
    fn degrees(v: u32, limit: u32) -> Option<f64> {
        let v = v & 0x0fff_ffff;
        let (d, m) = (v / 1_000_000, v % 1_000_000);
        if m >= 600_000 || d > limit || (d == limit && m != 0) {
            return None;
        }
        Some(d as f64 + m as f64 / 600_000.0)
    }
    let (lat, lon) = (degrees(lat, 90)?, -degrees(lon, 180)?);
    if (lat, lon) == (0.0, 0.0) {
        return None;
    }
    Some(Position { unit: p.source(), lat, lon, source: "HyteraDMRCompressed" })
}

/// Vendor words whose fields are not established. Preserve flags and raw
/// bytes rather than interpreting them as grants, radio IDs or positions.
pub fn control(e: &super::slot::SlotEvent) -> Option<String> {
    use super::slot::{FID_HYTERA, SlotEvent};
    match e {
        SlotEvent::Csbk { csbk, mbc: false } if csbk.fid() == FID_HYTERA && matches!(csbk.opcode(), 8 | 13) => {
            Some(format!("Hytera vendor CSBK opcode 0x{:02x} PF={} raw {}", csbk.opcode(), csbk.0[0] & 0x40 != 0, hex(&csbk.0)))
        }
        SlotEvent::Lc { lc, from } if lc.fid() == FID_HYTERA && matches!(lc.flco(), 13 | 48) => {
            Some(format!("Hytera vendor LC opcode 0x{:02x} {from:?} raw {}", lc.flco(), hex(&lc.0)))
        }
        SlotEvent::Slco(raw) if raw[0] >> 4 == 12 => Some(format!("DMR vendor short LC opcode 0x0c raw {}", hex(raw))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dmr::data::tests::{gps, rrs};
    fn fixture(h: [u8; 12], mut b: Vec<u8>) -> Packet {
        b.truncate(b.len() - 4 - (h[9] / 8) as usize);
        Packet { header: h, payload: b }
    }
    #[test]
    fn compressed_coordinates_are_degrees_and_minutes_not_microdegrees() {
        let (h, b) = gps();
        let p = fixture(h, b);
        let m = packet(&p);
        let pos = m.position.unwrap();
        assert_eq!((m.kind, pos.unit, pos.lat, pos.lon), (MessageType::Location, 123, 45.5, -90.25));
        assert_eq!(pos.source, "HyteraDMRCompressed");
        assert!(m.meta.contains("experimental N/W"));
    }
    #[test]
    fn unknown_flags_invalid_date_and_bad_coordinates_stay_raw() {
        let (h, b) = gps();
        let p = fixture(h, b);
        for (index, value) in [(18, 0x02), (18, 0xc2), (22, 0x05), (9, 24), (10, 60), (12, 0), (13, 13), (12, 32)] {
            let mut q = p.clone();
            q.payload[index] = value;
            assert!(packet(&q).position.is_none(), "index {index} value {value}");
            assert!(packet(&q).meta.contains(&hex(&q.payload)));
        }
        for (offset, v) in [(15, 0x8000_0000 | 45_600_000u32), (19, 0x8000_0000 | 181_000_000)] {
            let mut q = p.clone();
            q.payload[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
            assert!(packet(&q).position.is_none());
        }
        for len in 0..p.payload.len() {
            let mut q = p.clone();
            q.payload.truncate(len);
            assert!(packet(&q).position.is_none());
        }
    }
    #[test]
    fn rrs_checks_internal_checksum_and_keeps_sender_separate() {
        let (h, b) = rrs();
        let p = fixture(h, b);
        let m = packet(&p);
        assert_eq!((m.kind, m.source, m.position), (MessageType::Registration, 123, None));
        assert!(m.meta.contains("10.0.0.123"));
        let mut bad = p.clone();
        bad.payload[7] ^= 1;
        assert_eq!(packet(&bad).kind, MessageType::Status);
        let mut answer = p.clone();
        answer.payload = vec![2, 0, 0x11, 0, 0x80, 0, 9, 10, 0, 0, 123, 0, 0, 0, 3, 0x84];
        let sum = answer.payload[3..].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        answer.payload.extend([(!sum).wrapping_add(0x33), 3]);
        answer.header[5..8].copy_from_slice(&[0, 0, 16]);
        let m = packet(&answer);
        assert_eq!((m.kind, m.source), (MessageType::Acknowledge, 16));
        assert!(m.meta.contains("renewal 900 s") && m.meta.contains("10.0.0.123"));
    }
}
