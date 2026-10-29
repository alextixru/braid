//! NAT и таблица потоков — тикет 02.
//!
//! [`FlowTable`]: поток (5-tuple) закреплён за движком на всё время жизни;
//! исходящий пакет получает `src` = адрес движка в туннеле и, для fake-IP,
//! `dst` = настоящий адрес; входящий восстанавливает fake и адрес tun.
//! Контрольные суммы IPv4/TCP/UDP пересчитываются инкрементально
//! ([`checksum`]), ICMP echo не меняется. Смерть движка →
//! RST его TCP-потокам (ист. 19), остальные потоки забываются.

pub mod checksum;
mod flow;
pub mod packet;

pub use flow::{Decision, FlowTable, FlowTimeouts, Outbound};
