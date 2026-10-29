//! Общее для тестов роутера: сборка IPv4-пакетов и независимый подсчёт
//! контрольных сумм (не через код ядра — чтобы проверка была честной).

#![allow(dead_code)]

use std::net::Ipv4Addr;

pub const TUN_ADDR: Ipv4Addr = Ipv4Addr::new(10, 255, 0, 2);
pub const PROXY_ADDR: Ipv4Addr = Ipv4Addr::new(10, 255, 0, 1);

/// Сумма дополнений до единицы по RFC 1071, свёрнутая и инвертированная.
pub fn ones_complement(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in data.chunks(2) {
        let word = if chunk.len() == 2 {
            u16::from_be_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], 0])
        };
        sum += u32::from(word);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn pseudo_header(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, len: usize) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    p.push(0);
    p.push(proto);
    p.extend_from_slice(&(len as u16).to_be_bytes());
    p
}

/// IPv4-заголовок без опций, TTL 64, DF; `payload` — транспортный сегмент целиком.
pub fn ipv4(proto: u8, src: Ipv4Addr, dst: Ipv4Addr, payload: &[u8]) -> Vec<u8> {
    let total = 20 + payload.len();
    let mut p = vec![
        0x45,
        0,
        (total >> 8) as u8,
        total as u8,
        0,
        0,
        0x40,
        0,
        64,
        proto,
        0,
        0,
    ];
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    let c = ones_complement(&p);
    p[10..12].copy_from_slice(&c.to_be_bytes());
    p.extend_from_slice(payload);
    p
}

pub fn udp(src: Ipv4Addr, sport: u16, dst: Ipv4Addr, dport: u16, payload: &[u8]) -> Vec<u8> {
    let len = 8 + payload.len();
    let mut seg = Vec::with_capacity(len);
    seg.extend_from_slice(&sport.to_be_bytes());
    seg.extend_from_slice(&dport.to_be_bytes());
    seg.extend_from_slice(&(len as u16).to_be_bytes());
    seg.extend_from_slice(&[0, 0]);
    seg.extend_from_slice(payload);
    let mut data = pseudo_header(src, dst, 17, len);
    data.extend_from_slice(&seg);
    let mut c = ones_complement(&data);
    if c == 0 {
        c = 0xffff;
    }
    seg[6..8].copy_from_slice(&c.to_be_bytes());
    ipv4(17, src, dst, &seg)
}

/// UDP без контрольной суммы (поле 0 — разрешено для IPv4).
pub fn udp_no_checksum(
    src: Ipv4Addr,
    sport: u16,
    dst: Ipv4Addr,
    dport: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut p = udp(src, sport, dst, dport, payload);
    p[26..28].copy_from_slice(&[0, 0]);
    p
}

pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;
pub const ACK: u8 = 0x10;

#[allow(clippy::too_many_arguments)]
pub fn tcp(
    src: Ipv4Addr,
    sport: u16,
    dst: Ipv4Addr,
    dport: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut seg = Vec::with_capacity(20 + payload.len());
    seg.extend_from_slice(&sport.to_be_bytes());
    seg.extend_from_slice(&dport.to_be_bytes());
    seg.extend_from_slice(&seq.to_be_bytes());
    seg.extend_from_slice(&ack.to_be_bytes());
    seg.push(5 << 4);
    seg.push(flags);
    seg.extend_from_slice(&65535u16.to_be_bytes());
    seg.extend_from_slice(&[0, 0, 0, 0]);
    seg.extend_from_slice(payload);
    let mut data = pseudo_header(src, dst, 6, seg.len());
    data.extend_from_slice(&seg);
    let c = ones_complement(&data);
    seg[16..18].copy_from_slice(&c.to_be_bytes());
    ipv4(6, src, dst, &seg)
}

pub fn icmp_echo(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    kind: u8,
    id: u16,
    seq: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut m = vec![kind, 0, 0, 0];
    m.extend_from_slice(&id.to_be_bytes());
    m.extend_from_slice(&seq.to_be_bytes());
    m.extend_from_slice(payload);
    let c = ones_complement(&m);
    m[2..4].copy_from_slice(&c.to_be_bytes());
    ipv4(1, src, dst, &m)
}

/// Разобранный пакет для проверок.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub proto: u8,
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub sport: u16,
    pub dport: u16,
    pub payload: Vec<u8>,
}

pub fn parse(pkt: &[u8]) -> Parsed {
    assert!(pkt.len() >= 20, "короче IPv4-заголовка: {} байт", pkt.len());
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    let proto = pkt[9];
    let seg = &pkt[ihl..];
    let (sport, dport, payload) = match proto {
        6 => {
            let off = usize::from(seg[12] >> 4) * 4;
            (
                u16::from_be_bytes([seg[0], seg[1]]),
                u16::from_be_bytes([seg[2], seg[3]]),
                seg[off..].to_vec(),
            )
        }
        17 => (
            u16::from_be_bytes([seg[0], seg[1]]),
            u16::from_be_bytes([seg[2], seg[3]]),
            seg[8..].to_vec(),
        ),
        1 => {
            let id = u16::from_be_bytes([seg[4], seg[5]]);
            (id, id, seg[8..].to_vec())
        }
        _ => (0, 0, seg.to_vec()),
    };
    Parsed {
        proto,
        src: Ipv4Addr::from([pkt[12], pkt[13], pkt[14], pkt[15]]),
        dst: Ipv4Addr::from([pkt[16], pkt[17], pkt[18], pkt[19]]),
        sport,
        dport,
        payload,
    }
}

/// Все контрольные суммы пакета сходятся: заголовок IPv4 и транспорт
/// (TCP/UDP с псевдозаголовком, ICMP без него). UDP с нулевой суммой — верен.
pub fn checksums_ok(pkt: &[u8]) -> bool {
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    if ones_complement(&pkt[..ihl]) != 0 {
        return false;
    }
    let src = Ipv4Addr::from([pkt[12], pkt[13], pkt[14], pkt[15]]);
    let dst = Ipv4Addr::from([pkt[16], pkt[17], pkt[18], pkt[19]]);
    let proto = pkt[9];
    let seg = &pkt[ihl..];
    match proto {
        6 | 17 => {
            if proto == 17 && seg[6..8] == [0, 0] {
                return true;
            }
            let mut data = pseudo_header(src, dst, proto, seg.len());
            data.extend_from_slice(seg);
            ones_complement(&data) == 0
        }
        1 => ones_complement(seg) == 0,
        _ => true,
    }
}

pub fn tcp_flags(pkt: &[u8]) -> u8 {
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    pkt[ihl + 13]
}

pub fn tcp_seq(pkt: &[u8]) -> u32 {
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    u32::from_be_bytes([pkt[ihl + 4], pkt[ihl + 5], pkt[ihl + 6], pkt[ihl + 7]])
}

pub fn tcp_ack(pkt: &[u8]) -> u32 {
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    u32::from_be_bytes([pkt[ihl + 8], pkt[ihl + 9], pkt[ihl + 10], pkt[ihl + 11]])
}

/// Минимальный DNS-запрос (один вопрос, RD).
pub fn dns_query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut q = Vec::new();
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&[0, 1]);
    q
}

/// Разобранный DNS-ответ: код ответа и записи A (адрес, TTL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsReply {
    pub id: u16,
    pub is_response: bool,
    pub rcode: u8,
    pub answers: u16,
    pub a: Vec<(Ipv4Addr, u32)>,
}

fn skip_name(msg: &[u8], mut pos: usize) -> usize {
    loop {
        let len = msg[pos];
        if len == 0 {
            return pos + 1;
        }
        if len & 0xc0 == 0xc0 {
            return pos + 2;
        }
        pos += 1 + usize::from(len);
    }
}

pub fn parse_dns(msg: &[u8]) -> DnsReply {
    assert!(msg.len() >= 12, "короче DNS-заголовка");
    let id = u16::from_be_bytes([msg[0], msg[1]]);
    let is_response = msg[2] & 0x80 != 0;
    let rcode = msg[3] & 0x0f;
    let qd = u16::from_be_bytes([msg[4], msg[5]]);
    let an = u16::from_be_bytes([msg[6], msg[7]]);
    let mut pos = 12;
    for _ in 0..qd {
        pos = skip_name(msg, pos) + 4;
    }
    let mut a = Vec::new();
    for _ in 0..an {
        pos = skip_name(msg, pos);
        let rtype = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
        let ttl = u32::from_be_bytes([msg[pos + 4], msg[pos + 5], msg[pos + 6], msg[pos + 7]]);
        let rdlen = usize::from(u16::from_be_bytes([msg[pos + 8], msg[pos + 9]]));
        let rdata = &msg[pos + 10..pos + 10 + rdlen];
        if rtype == 1 && rdlen == 4 {
            a.push((
                Ipv4Addr::from([rdata[0], rdata[1], rdata[2], rdata[3]]),
                ttl,
            ));
        }
        pos += 10 + rdlen;
    }
    DnsReply {
        id,
        is_response,
        rcode,
        answers: an,
        a,
    }
}

/// Ответ с одной записью A на запрос `query` — «подделка upstream».
pub fn dns_answer_a(query: &[u8], ip: Ipv4Addr, ttl: u32) -> Vec<u8> {
    let mut r = query.to_vec();
    r[2] |= 0x80; // QR
    r[3] = 0x80; // RA, RCODE 0
    r[6..8].copy_from_slice(&1u16.to_be_bytes());
    r.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1]);
    r.extend_from_slice(&ttl.to_be_bytes());
    r.extend_from_slice(&[0, 4]);
    r.extend_from_slice(&ip.octets());
    r
}

pub fn in_fake_range(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 198 && (o[1] == 18 || o[1] == 19)
}
