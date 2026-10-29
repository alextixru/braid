//! Разбор и сборка IPv4-пакетов: заголовок, порты TCP/UDP, идентификатор
//! ICMP echo, подмена адресов и портов с инкрементальным пересчётом сумм,
//! сборка UDP/TCP/ICMP-пакетов с полным подсчётом.

use super::checksum;
use std::net::Ipv4Addr;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

pub const ICMP_ECHO_REPLY: u8 = 0;
pub const ICMP_ECHO_REQUEST: u8 = 8;

pub const TCP_FIN: u8 = 0x01;
pub const TCP_SYN: u8 = 0x02;
pub const TCP_RST: u8 = 0x04;
pub const TCP_PSH: u8 = 0x08;
pub const TCP_ACK: u8 = 0x10;

/// Версия IP по первому байту; `None` — пусто.
pub fn ip_version(pkt: &[u8]) -> Option<u8> {
    pkt.first().map(|b| b >> 4)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Header {
    /// Длина заголовка в байтах (20 без опций).
    pub ihl: usize,
    /// Длина пакета по заголовку; хвост буфера за ней — не пакет.
    pub total_len: usize,
    pub proto: u8,
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub more_fragments: bool,
    /// В восьмибайтных блоках; `0` — первый (или единственный) фрагмент.
    pub fragment_offset: u16,
}

impl Ipv4Header {
    pub fn parse(pkt: &[u8]) -> Option<Self> {
        if pkt.len() < 20 || pkt[0] >> 4 != 4 {
            return None;
        }
        let ihl = usize::from(pkt[0] & 0x0f) * 4;
        let total_len = usize::from(u16::from_be_bytes([pkt[2], pkt[3]]));
        if ihl < 20 || total_len < ihl || pkt.len() < total_len {
            return None;
        }
        let frag = u16::from_be_bytes([pkt[6], pkt[7]]);
        Some(Self {
            ihl,
            total_len,
            proto: pkt[9],
            src: Ipv4Addr::from([pkt[12], pkt[13], pkt[14], pkt[15]]),
            dst: Ipv4Addr::from([pkt[16], pkt[17], pkt[18], pkt[19]]),
            more_fragments: frag & 0x2000 != 0,
            fragment_offset: frag & 0x1fff,
        })
    }

    /// Транспортный сегмент (без заголовка IP и без хвоста буфера).
    pub fn payload<'a>(&self, pkt: &'a [u8]) -> &'a [u8] {
        &pkt[self.ihl..self.total_len]
    }

    /// У первого фрагмента есть транспортный заголовок; у остальных — нет.
    pub fn has_transport_header(&self) -> bool {
        self.fragment_offset == 0
    }
}

/// Порты TCP/UDP или идентификатор ICMP echo (в обоих полях).
/// `None` — не первый фрагмент, обрубок или протокол без портов.
pub fn ports(h: &Ipv4Header, pkt: &[u8]) -> Option<(u16, u16)> {
    if !h.has_transport_header() {
        return None;
    }
    let seg = h.payload(pkt);
    match h.proto {
        PROTO_TCP | PROTO_UDP => {
            if seg.len() < 8 {
                return None;
            }
            Some((
                u16::from_be_bytes([seg[0], seg[1]]),
                u16::from_be_bytes([seg[2], seg[3]]),
            ))
        }
        PROTO_ICMP => {
            if seg.len() < 8 || !matches!(seg[0], ICMP_ECHO_REQUEST | ICMP_ECHO_REPLY) {
                return None;
            }
            let id = u16::from_be_bytes([seg[4], seg[5]]);
            Some((id, id))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpInfo {
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    /// Длина заголовка TCP в байтах.
    pub header_len: usize,
    pub payload_len: usize,
}

impl TcpInfo {
    pub fn parse(h: &Ipv4Header, pkt: &[u8]) -> Option<Self> {
        if h.proto != PROTO_TCP || !h.has_transport_header() {
            return None;
        }
        let seg = h.payload(pkt);
        if seg.len() < 20 {
            return None;
        }
        let header_len = usize::from(seg[12] >> 4) * 4;
        if header_len < 20 || seg.len() < header_len {
            return None;
        }
        Some(Self {
            seq: u32::from_be_bytes([seg[4], seg[5], seg[6], seg[7]]),
            ack: u32::from_be_bytes([seg[8], seg[9], seg[10], seg[11]]),
            flags: seg[13],
            window: u16::from_be_bytes([seg[14], seg[15]]),
            header_len,
            payload_len: seg.len() - header_len,
        })
    }

    /// Сколько номеров последовательности занимает сегмент (SYN и FIN — по одному).
    pub fn seq_len(&self) -> u32 {
        let mut n = self.payload_len as u32;
        if self.flags & TCP_SYN != 0 {
            n += 1;
        }
        if self.flags & TCP_FIN != 0 {
            n += 1;
        }
        n
    }
}

/// Смещение поля контрольной суммы транспорта от начала пакета, если оно есть
/// и его надо пересчитывать (UDP с нулём — нет).
fn transport_csum_at(h: &Ipv4Header, pkt: &[u8]) -> Option<usize> {
    if !h.has_transport_header() {
        return None;
    }
    let at = match h.proto {
        PROTO_TCP => h.ihl + 16,
        PROTO_UDP => h.ihl + 6,
        _ => return None,
    };
    if at + 2 > h.total_len {
        return None;
    }
    if h.proto == PROTO_UDP && pkt[at..at + 2] == [0, 0] {
        return None;
    }
    Some(at)
}

fn patch(pkt: &mut [u8], csum_at: usize, old: &[u8], new: &[u8]) {
    let cur = u16::from_be_bytes([pkt[csum_at], pkt[csum_at + 1]]);
    let next = checksum::update(cur, old, new);
    pkt[csum_at..csum_at + 2].copy_from_slice(&next.to_be_bytes());
}

fn set_addr(pkt: &mut [u8], h: &Ipv4Header, at: usize, new: Ipv4Addr) {
    let old: [u8; 4] = pkt[at..at + 4].try_into().expect("4 байта адреса");
    let new = new.octets();
    if old == new {
        return;
    }
    patch(pkt, 10, &old, &new);
    if let Some(t) = transport_csum_at(h, pkt) {
        patch(pkt, t, &old, &new);
        // UDP: нулевая сумма значит «нет суммы», поэтому результат 0 пишется как 0xffff.
        if h.proto == PROTO_UDP && pkt[t..t + 2] == [0, 0] {
            pkt[t..t + 2].copy_from_slice(&[0xff, 0xff]);
        }
    }
    pkt[at..at + 4].copy_from_slice(&new);
}

/// Подменить адрес источника с пересчётом сумм IPv4 и TCP/UDP.
pub fn set_src(pkt: &mut [u8], h: &Ipv4Header, new: Ipv4Addr) {
    set_addr(pkt, h, 12, new);
}

pub fn set_dst(pkt: &mut [u8], h: &Ipv4Header, new: Ipv4Addr) {
    set_addr(pkt, h, 16, new);
}

fn set_port(pkt: &mut [u8], h: &Ipv4Header, at: usize, new: u16) {
    if !matches!(h.proto, PROTO_TCP | PROTO_UDP) || !h.has_transport_header() {
        return;
    }
    let old: [u8; 2] = pkt[at..at + 2].try_into().expect("2 байта порта");
    let new = new.to_be_bytes();
    if old == new {
        return;
    }
    if let Some(t) = transport_csum_at(h, pkt) {
        patch(pkt, t, &old, &new);
        if h.proto == PROTO_UDP && pkt[t..t + 2] == [0, 0] {
            pkt[t..t + 2].copy_from_slice(&[0xff, 0xff]);
        }
    }
    pkt[at..at + 2].copy_from_slice(&new);
}

/// Подменить порт источника TCP/UDP с пересчётом суммы транспорта.
pub fn set_sport(pkt: &mut [u8], h: &Ipv4Header, new: u16) {
    set_port(pkt, h, h.ihl, new);
}

pub fn set_dport(pkt: &mut [u8], h: &Ipv4Header, new: u16) {
    set_port(pkt, h, h.ihl + 2, new);
}

/// IPv4 без опций: TTL 64, DF, идентификатор 0; `segment` — транспорт целиком.
pub fn build_ipv4(proto: u8, src: Ipv4Addr, dst: Ipv4Addr, segment: &[u8]) -> Vec<u8> {
    let total = 20 + segment.len();
    let mut p = Vec::with_capacity(total);
    p.extend_from_slice(&[0x45, 0]);
    p.extend_from_slice(&(total as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0, 0x40, 0, 64, proto, 0, 0]);
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    let c = checksum::ipv4(&p);
    p[10..12].copy_from_slice(&c.to_be_bytes());
    p.extend_from_slice(segment);
    p
}

pub fn build_udp(src: Ipv4Addr, sport: u16, dst: Ipv4Addr, dport: u16, payload: &[u8]) -> Vec<u8> {
    let len = 8 + payload.len();
    let mut seg = Vec::with_capacity(len);
    seg.extend_from_slice(&sport.to_be_bytes());
    seg.extend_from_slice(&dport.to_be_bytes());
    seg.extend_from_slice(&(len as u16).to_be_bytes());
    seg.extend_from_slice(&[0, 0]);
    seg.extend_from_slice(payload);
    let mut c = checksum::transport(src, dst, PROTO_UDP, &seg, 6);
    if c == 0 {
        c = 0xffff;
    }
    seg[6..8].copy_from_slice(&c.to_be_bytes());
    build_ipv4(PROTO_UDP, src, dst, &seg)
}

/// Сегмент TCP; `options` — уже выровненные до 4 байт.
#[allow(clippy::too_many_arguments)]
pub fn build_tcp(
    src: Ipv4Addr,
    sport: u16,
    dst: Ipv4Addr,
    dport: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    options: &[u8],
    payload: &[u8],
) -> Vec<u8> {
    debug_assert_eq!(options.len() % 4, 0);
    let header_len = 20 + options.len();
    let mut seg = Vec::with_capacity(header_len + payload.len());
    seg.extend_from_slice(&sport.to_be_bytes());
    seg.extend_from_slice(&dport.to_be_bytes());
    seg.extend_from_slice(&seq.to_be_bytes());
    seg.extend_from_slice(&ack.to_be_bytes());
    seg.push(((header_len / 4) as u8) << 4);
    seg.push(flags);
    seg.extend_from_slice(&window.to_be_bytes());
    seg.extend_from_slice(&[0, 0, 0, 0]);
    seg.extend_from_slice(options);
    seg.extend_from_slice(payload);
    let c = checksum::transport(src, dst, PROTO_TCP, &seg, 16);
    seg[16..18].copy_from_slice(&c.to_be_bytes());
    build_ipv4(PROTO_TCP, src, dst, &seg)
}

/// ICMP echo reply на запрос `pkt`: адреса местами, тип 0, остальное как в запросе.
pub fn build_icmp_echo_reply(h: &Ipv4Header, pkt: &[u8]) -> Option<Vec<u8>> {
    let req = h.payload(pkt);
    if h.proto != PROTO_ICMP || req.len() < 8 || req[0] != ICMP_ECHO_REQUEST {
        return None;
    }
    let mut msg = req.to_vec();
    msg[0] = ICMP_ECHO_REPLY;
    msg[1] = 0;
    let c = checksum::icmp(&msg);
    msg[2..4].copy_from_slice(&c.to_be_bytes());
    Some(build_ipv4(PROTO_ICMP, h.dst, h.src, &msg))
}
