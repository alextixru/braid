//! DNS по TCP/53: минимальный TCP-ответчик на адресе прокси. Умеет ровно то,
//! что нужно резолверу: рукопожатие, приём запросов с 2-байтной длиной
//! (в том числе кусками), ответ, закрытие по FIN клиента, RST чужим
//! сегментам. Повторной передачи нет: путь через tun не теряет пакеты.

use crate::nat::packet::{self, Ipv4Header, TCP_ACK, TCP_FIN, TCP_PSH, TCP_RST, TCP_SYN, TcpInfo};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

/// MSS, которое объявляем в SYN|ACK: укладывается в MTU 1300 CSQTT.
const MSS: usize = 1220;
const MSS_OPTION: [u8; 4] = [2, 4, (MSS >> 8) as u8, MSS as u8];
const WINDOW: u16 = 65535;
/// Соединение без сегментов столько времени забывается.
const IDLE: Duration = Duration::from_secs(30);
const MAX_CONNS: usize = 256;
/// DNS-сообщение по TCP не длиннее поля длины.
const MAX_MESSAGE: usize = 65535;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnKey {
    pub client: Ipv4Addr,
    pub port: u16,
}

/// Что сделать после сегмента: записать пакеты в tun и обработать запросы.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TcpStep {
    pub out: Vec<Vec<u8>>,
    pub queries: Vec<(ConnKey, Vec<u8>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    SynReceived,
    Established,
    /// Клиент прислал FIN, наш FIN ушёл — ждём ACK на него.
    Closing,
}

#[derive(Debug)]
struct Conn {
    state: State,
    snd_nxt: u32,
    rcv_nxt: u32,
    inbuf: Vec<u8>,
    last_seen: Instant,
}

pub struct TcpDns {
    server: Ipv4Addr,
    port: u16,
    conns: HashMap<ConnKey, Conn>,
    next_isn: u32,
}

#[allow(clippy::too_many_arguments)]
fn segment_out(
    server: Ipv4Addr,
    port: u16,
    key: &ConnKey,
    seq: u32,
    ack: u32,
    flags: u8,
    options: &[u8],
    payload: &[u8],
) -> Vec<u8> {
    packet::build_tcp(
        server, port, key.client, key.port, seq, ack, flags, WINDOW, options, payload,
    )
}

impl TcpDns {
    pub fn new(server: Ipv4Addr, port: u16) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(1);
        Self {
            server,
            port,
            conns: HashMap::new(),
            next_isn: seed,
        }
    }

    /// Сегмент от клиента (целый IPv4-пакет). Не наш адрес или порт — пусто.
    pub fn segment(&mut self, pkt: &[u8], now: Instant) -> TcpStep {
        let mut step = TcpStep::default();
        let Some(h) = Ipv4Header::parse(pkt) else {
            return step;
        };
        let Some(t) = TcpInfo::parse(&h, pkt) else {
            return step;
        };
        let Some((sport, dport)) = packet::ports(&h, pkt) else {
            return step;
        };
        if h.dst != self.server || dport != self.port {
            return step;
        }
        let (server, port) = (self.server, self.port);
        let key = ConnKey {
            client: h.src,
            port: sport,
        };
        let payload = &h.payload(pkt)[t.header_len..];

        if t.flags & TCP_RST != 0 {
            self.conns.remove(&key);
            return step;
        }
        if t.flags & TCP_SYN != 0 && t.flags & TCP_ACK == 0 {
            if self.conns.len() >= MAX_CONNS && !self.conns.contains_key(&key) {
                self.evict_oldest();
            }
            let isn = self.next_isn;
            self.next_isn = self.next_isn.wrapping_add(64_000);
            let rcv_nxt = t.seq.wrapping_add(1);
            self.conns.insert(
                key,
                Conn {
                    state: State::SynReceived,
                    snd_nxt: isn.wrapping_add(1),
                    rcv_nxt,
                    inbuf: Vec::new(),
                    last_seen: now,
                },
            );
            step.out.push(segment_out(
                server,
                port,
                &key,
                isn,
                rcv_nxt,
                TCP_SYN | TCP_ACK,
                &MSS_OPTION,
                &[],
            ));
            return step;
        }
        let Some(conn) = self.conns.get_mut(&key) else {
            // Сегмент без рукопожатия → RST (RFC 793 §3.4).
            let (seq, ack, flags) = if t.flags & TCP_ACK != 0 {
                (t.ack, 0, TCP_RST)
            } else {
                (0, t.seq.wrapping_add(t.seq_len()), TCP_RST | TCP_ACK)
            };
            step.out
                .push(segment_out(server, port, &key, seq, ack, flags, &[], &[]));
            return step;
        };
        conn.last_seen = now;
        if conn.state == State::SynReceived && t.flags & TCP_ACK != 0 && t.ack == conn.snd_nxt {
            conn.state = State::Established;
        }
        let mut need_ack = false;
        if !payload.is_empty() {
            if t.seq == conn.rcv_nxt {
                conn.inbuf.extend_from_slice(payload);
                conn.rcv_nxt = conn.rcv_nxt.wrapping_add(payload.len() as u32);
                while conn.inbuf.len() >= 2 {
                    let len = usize::from(u16::from_be_bytes([conn.inbuf[0], conn.inbuf[1]]));
                    if conn.inbuf.len() < 2 + len {
                        break;
                    }
                    let query = conn.inbuf[2..2 + len].to_vec();
                    conn.inbuf.drain(..2 + len);
                    step.queries.push((key, query));
                }
            }
            // Дубликат или дыра — тоже ACK с тем, что ждём.
            need_ack = true;
        }
        if t.flags & TCP_FIN != 0 && t.seq.wrapping_add(payload.len() as u32) == conn.rcv_nxt {
            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
            let seq = conn.snd_nxt;
            conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
            conn.state = State::Closing;
            step.out.push(segment_out(
                server,
                port,
                &key,
                seq,
                conn.rcv_nxt,
                TCP_FIN | TCP_ACK,
                &[],
                &[],
            ));
            need_ack = false;
        }
        if need_ack {
            step.out.push(segment_out(
                server,
                port,
                &key,
                conn.snd_nxt,
                conn.rcv_nxt,
                TCP_ACK,
                &[],
                &[],
            ));
        }
        if conn.state == State::Closing && t.flags & TCP_ACK != 0 && t.ack == conn.snd_nxt {
            self.conns.remove(&key);
        }
        step
    }

    /// Ответ клиенту: длина + сообщение, кусками по MSS.
    pub fn respond(&mut self, key: &ConnKey, answer: &[u8], now: Instant) -> Vec<Vec<u8>> {
        let (server, port) = (self.server, self.port);
        let Some(conn) = self.conns.get_mut(key) else {
            return Vec::new();
        };
        conn.last_seen = now;
        let answer = &answer[..answer.len().min(MAX_MESSAGE)];
        let mut data = Vec::with_capacity(2 + answer.len());
        data.extend_from_slice(&(answer.len() as u16).to_be_bytes());
        data.extend_from_slice(answer);
        let mut out = Vec::new();
        for chunk in data.chunks(MSS) {
            let seq = conn.snd_nxt;
            conn.snd_nxt = conn.snd_nxt.wrapping_add(chunk.len() as u32);
            out.push(segment_out(
                server,
                port,
                key,
                seq,
                conn.rcv_nxt,
                TCP_PSH | TCP_ACK,
                &[],
                chunk,
            ));
        }
        out
    }

    /// Забыть соединения без сегментов дольше 30 с; возвращает, сколько.
    pub fn gc(&mut self, now: Instant) -> usize {
        let before = self.conns.len();
        self.conns
            .retain(|_, c| now.saturating_duration_since(c.last_seen) <= IDLE);
        before - self.conns.len()
    }

    fn evict_oldest(&mut self) {
        if let Some(key) = self
            .conns
            .iter()
            .min_by_key(|(_, c)| c.last_seen)
            .map(|(k, _)| *k)
        {
            self.conns.remove(&key);
        }
    }

    pub fn len(&self) -> usize {
        self.conns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.conns.is_empty()
    }
}
