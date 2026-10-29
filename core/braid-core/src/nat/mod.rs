//! NAT и таблица потоков — тикет 02.
//!
//! `FlowTable { outbound(pkt, decision) -> Option<Pkt>, inbound(engine, pkt) -> Option<Pkt>,
//! drop_engine(engine) -> Vec<RstPkt>, gc(now) }`. Поток (5-tuple) закреплён за
//! движком на всё время жизни; исходящий — `src` = адрес движка, `dst` = настоящий
//! IP вместо fake; входящий восстанавливает fake и адрес tun. Контрольные суммы
//! IPv4/TCP/UDP/ICMP пересчитываются инкрементально. Смерть движка → RST его
//! TCP-потокам (ист. 19).
