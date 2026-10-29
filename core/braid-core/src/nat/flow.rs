//! Таблица потоков. Поток — 5-tuple исходящего пакета (протокол, адреса,
//! порты; у ICMP echo — идентификатор в обоих портах). Порт источника
//! сохраняется, пока два потока не сошлись на одном настоящем адресе с одним
//! портом (два fake-IP одного CDN) — тогда второму выдаётся порт из диапазона
//! NAT. Идентификаторы ICMP не переписываются.

use super::packet::{
    self, Ipv4Header, PROTO_ICMP, PROTO_TCP, PROTO_UDP, TCP_ACK, TCP_FIN, TCP_RST, TcpInfo,
};
use braid_proto::EngineId;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

const NAT_PORT_FIRST: u16 = 32768;
const NAT_PORT_LAST: u16 = 60999;

/// Куда и как отправить новый поток.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// `dst` — fake-IP: `src` → адрес движка, `dst` → настоящий адрес.
    FakeIp {
        engine: EngineId,
        engine_ip: Ipv4Addr,
        real_dst: Ipv4Addr,
    },
    /// `dst` настоящий (подсеть правил, режим «по приложениям»): меняется только `src`.
    Real {
        engine: EngineId,
        engine_ip: Ipv4Addr,
    },
}

/// Пакет после NAT и движок, которому он принадлежит.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbound {
    pub engine: EngineId,
    pub pkt: Vec<u8>,
}

/// Таймауты простоя по протоколам.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowTimeouts {
    /// TCP после ответа сервера и до первого FIN.
    pub tcp_established: Duration,
    /// TCP до ответа сервера или после FIN/RST.
    pub tcp_transient: Duration,
    pub udp: Duration,
    pub icmp: Duration,
}

impl Default for FlowTimeouts {
    fn default() -> Self {
        Self {
            tcp_established: Duration::from_secs(3600),
            tcp_transient: Duration::from_secs(60),
            udp: Duration::from_secs(120),
            icmp: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ForwardKey {
    proto: u8,
    src: Ipv4Addr,
    sport: u16,
    dst: Ipv4Addr,
    dport: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ReverseKey {
    proto: u8,
    remote: Ipv4Addr,
    rport: u16,
    lport: u16,
}

#[derive(Debug, Default)]
struct TcpState {
    /// Сервер ответил хоть чем-то с ACK.
    established: bool,
    client_fin: bool,
    server_fin: bool,
    /// Следующий номер клиента и его последний ACK — для RST (RFC 5961: `seq == RCV.NXT`).
    client_seq_next: u32,
    client_ack: u32,
}

#[derive(Debug)]
struct Flow {
    key: ForwardKey,
    engine: EngineId,
    engine_ip: Ipv4Addr,
    real_dst: Ipv4Addr,
    nat_sport: u16,
    last_seen: Instant,
    tcp: TcpState,
}

impl Flow {
    fn idle_limit(&self, t: &FlowTimeouts) -> Duration {
        match self.key.proto {
            PROTO_TCP => {
                if self.tcp.established && !self.tcp.client_fin && !self.tcp.server_fin {
                    t.tcp_established
                } else {
                    t.tcp_transient
                }
            }
            PROTO_UDP => t.udp,
            _ => t.icmp,
        }
    }
}

pub struct FlowTable {
    timeouts: FlowTimeouts,
    flows: HashMap<ForwardKey, Flow>,
    reverse: HashMap<ReverseKey, ForwardKey>,
    next_port: u16,
}

impl FlowTable {
    pub fn new(timeouts: FlowTimeouts) -> Self {
        Self {
            timeouts,
            flows: HashMap::new(),
            reverse: HashMap::new(),
            next_port: NAT_PORT_FIRST,
        }
    }

    /// Пакет из tun → пакет для движка. Существующий поток идёт в свой движок,
    /// какое бы решение ни пришло; новый создаётся только по решению.
    pub fn outbound(
        &mut self,
        pkt: &[u8],
        decision: Option<&Decision>,
        now: Instant,
    ) -> Option<Outbound> {
        self.outbound_with(pkt, now, || decision.cloned())
    }

    /// То же, но решение вычисляется лениво — только для нового потока.
    pub fn outbound_with<F>(&mut self, pkt: &[u8], now: Instant, decide: F) -> Option<Outbound>
    where
        F: FnOnce() -> Option<Decision>,
    {
        let h = Ipv4Header::parse(pkt)?;
        let (sport, dport) = packet::ports(&h, pkt)?;
        let key = ForwardKey {
            proto: h.proto,
            src: h.src,
            sport,
            dst: h.dst,
            dport,
        };
        if !self.flows.contains_key(&key) {
            let (engine, engine_ip, real_dst) = match decide()? {
                Decision::FakeIp {
                    engine,
                    engine_ip,
                    real_dst,
                } => (engine, engine_ip, real_dst),
                Decision::Real { engine, engine_ip } => (engine, engine_ip, h.dst),
            };
            let nat_sport = self.pick_port(h.proto, real_dst, dport, sport)?;
            self.reverse.insert(
                ReverseKey {
                    proto: h.proto,
                    remote: real_dst,
                    rport: dport,
                    lport: nat_sport,
                },
                key,
            );
            self.flows.insert(
                key,
                Flow {
                    key,
                    engine,
                    engine_ip,
                    real_dst,
                    nat_sport,
                    last_seen: now,
                    tcp: TcpState::default(),
                },
            );
        }
        let tcp = TcpInfo::parse(&h, pkt);
        let flow = self.flows.get_mut(&key)?;
        flow.last_seen = now;
        let mut out = pkt[..h.total_len].to_vec();
        packet::set_src(&mut out, &h, flow.engine_ip);
        if flow.real_dst != h.dst {
            packet::set_dst(&mut out, &h, flow.real_dst);
        }
        if flow.nat_sport != sport {
            packet::set_sport(&mut out, &h, flow.nat_sport);
        }
        let engine = flow.engine.clone();
        let mut reset = false;
        if let Some(t) = tcp {
            if t.flags & TCP_RST != 0 {
                reset = true;
            } else {
                flow.tcp.client_seq_next = t.seq.wrapping_add(t.seq_len());
                if t.flags & TCP_ACK != 0 {
                    flow.tcp.client_ack = t.ack;
                }
                if t.flags & TCP_FIN != 0 {
                    flow.tcp.client_fin = true;
                }
            }
        }
        if reset {
            self.remove(&key);
        }
        Some(Outbound { engine, pkt: out })
    }

    /// Пакет от движка → пакет для tun. Принимается только по потоку этого
    /// движка и на его адрес.
    pub fn inbound(&mut self, engine: &EngineId, pkt: &[u8], now: Instant) -> Option<Vec<u8>> {
        let h = Ipv4Header::parse(pkt)?;
        let (rport, lport) = packet::ports(&h, pkt)?;
        let key = *self.reverse.get(&ReverseKey {
            proto: h.proto,
            remote: h.src,
            rport,
            lport,
        })?;
        let tcp = TcpInfo::parse(&h, pkt);
        let flow = self.flows.get_mut(&key)?;
        if flow.engine != *engine || flow.engine_ip != h.dst {
            return None;
        }
        flow.last_seen = now;
        let mut out = pkt[..h.total_len].to_vec();
        packet::set_src(&mut out, &h, key.dst);
        packet::set_dst(&mut out, &h, key.src);
        if flow.nat_sport != key.sport {
            packet::set_dport(&mut out, &h, key.sport);
        }
        let mut reset = false;
        if let Some(t) = tcp {
            if t.flags & TCP_RST != 0 {
                reset = true;
            } else {
                if t.flags & TCP_ACK != 0 {
                    flow.tcp.established = true;
                }
                if t.flags & TCP_FIN != 0 {
                    flow.tcp.server_fin = true;
                }
            }
        }
        if reset {
            self.remove(&key);
        }
        Some(out)
    }

    /// Движок умер: его потоки забываются, TCP-потокам — RST в сторону клиента,
    /// чтобы приложения переподключились сразу (ист. 19).
    pub fn drop_engine(&mut self, engine: &EngineId) -> Vec<Vec<u8>> {
        let keys: Vec<ForwardKey> = self
            .flows
            .values()
            .filter(|f| f.engine == *engine)
            .map(|f| f.key)
            .collect();
        let mut rsts = Vec::new();
        for key in keys {
            let Some(flow) = self.remove(&key) else {
                continue;
            };
            if key.proto == PROTO_TCP && !(flow.tcp.client_fin && flow.tcp.server_fin) {
                rsts.push(packet::build_tcp(
                    key.dst,
                    key.dport,
                    key.src,
                    key.sport,
                    flow.tcp.client_ack,
                    flow.tcp.client_seq_next,
                    TCP_RST | TCP_ACK,
                    0,
                    &[],
                    &[],
                ));
            }
        }
        rsts
    }

    /// Убрать простаивающие потоки; возвращает, сколько убрано.
    pub fn gc(&mut self, now: Instant) -> usize {
        let timeouts = self.timeouts;
        let expired: Vec<ForwardKey> = self
            .flows
            .values()
            .filter(|f| now.saturating_duration_since(f.last_seen) > f.idle_limit(&timeouts))
            .map(|f| f.key)
            .collect();
        for key in &expired {
            self.remove(key);
        }
        expired.len()
    }

    pub fn len(&self) -> usize {
        self.flows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }

    fn remove(&mut self, key: &ForwardKey) -> Option<Flow> {
        let flow = self.flows.remove(key)?;
        self.reverse.remove(&ReverseKey {
            proto: key.proto,
            remote: flow.real_dst,
            rport: key.dport,
            lport: flow.nat_sport,
        });
        Some(flow)
    }

    fn port_free(&self, proto: u8, remote: Ipv4Addr, rport: u16, lport: u16) -> bool {
        !self.reverse.contains_key(&ReverseKey {
            proto,
            remote,
            rport,
            lport,
        })
    }

    /// Порт источника для нового потока: свой, если свободен, иначе из диапазона NAT.
    fn pick_port(&mut self, proto: u8, remote: Ipv4Addr, rport: u16, wanted: u16) -> Option<u16> {
        if self.port_free(proto, remote, rport, wanted) {
            return Some(wanted);
        }
        if proto == PROTO_ICMP {
            return None;
        }
        for _ in NAT_PORT_FIRST..=NAT_PORT_LAST {
            let port = self.next_port;
            self.next_port = if port >= NAT_PORT_LAST {
                NAT_PORT_FIRST
            } else {
                port + 1
            };
            if self.port_free(proto, remote, rport, port) {
                return Some(port);
            }
        }
        None
    }
}
