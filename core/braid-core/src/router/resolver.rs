//! Настоящий резолвер роутера: напрямую — UDP-сокеты к исходным резолверам
//! системы (на Android сокет защищается `Platform::protect_socket`); через
//! туннель — UDP-пакет от имени прокси в активный движок к его резолверу,
//! ответ приходит обратно через NAT и ловится по порту.

use super::{DNS_PORT, Shared};
use crate::dns::message::transaction_id;
use crate::dns::{ResolveFuture, Resolver, Via};
use crate::nat::packet;
use braid_platform::Platform;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

pub(crate) struct NetResolver {
    shared: Weak<Shared>,
    platform: Arc<dyn Platform>,
    timeout: Duration,
    upstream_port: u16,
}

impl NetResolver {
    pub(crate) fn new(
        shared: Weak<Shared>,
        platform: Arc<dyn Platform>,
        timeout: Duration,
        upstream_port: u16,
    ) -> Self {
        Self {
            shared,
            platform,
            timeout,
            upstream_port,
        }
    }

    /// Запрос всем исходным резолверам сразу; первый ответ с нашим
    /// идентификатором — наш.
    async fn direct(&self, query: &[u8]) -> Option<Vec<u8>> {
        let id = transaction_id(query)?;
        let servers = self.platform.original_dns();
        if servers.is_empty() {
            return None;
        }
        let deadline = tokio::time::Instant::now() + self.timeout;
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(8);
        let mut sent = false;
        for v4 in [true, false] {
            let targets: Vec<SocketAddr> = servers
                .iter()
                .filter(|s| s.is_ipv4() == v4)
                .map(|s| SocketAddr::new(*s, self.upstream_port))
                .collect();
            if targets.is_empty() {
                continue;
            }
            let bind: SocketAddr = if v4 {
                SocketAddr::from(([0, 0, 0, 0], 0))
            } else {
                SocketAddr::from(([0u16; 8], 0))
            };
            let Ok(sock) = UdpSocket::bind(bind).await else {
                continue;
            };
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                if !self.platform.protect_socket(sock.as_raw_fd()) {
                    continue;
                }
            }
            for target in &targets {
                if sock.send_to(query, target).await.is_ok() {
                    sent = true;
                }
            }
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                while let Ok(Ok((n, _))) =
                    tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await
                {
                    if tx.send(buf[..n].to_vec()).await.is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        if !sent {
            return None;
        }
        tokio::time::timeout_at(deadline, async {
            while let Some(reply) = rx.recv().await {
                let is_response = reply.get(2).is_some_and(|b| b & 0x80 != 0);
                if transaction_id(&reply) == Some(id) && is_response {
                    return Some(reply);
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
    }

    /// Запрос через активный движок к его резолверу (или `tunnel_dns` конфига).
    async fn tunnel(&self, query: &[u8]) -> Option<Vec<u8>> {
        let shared = self.shared.upgrade()?;
        let id = transaction_id(query)?;
        let (engine, engine_ip, servers) = shared.active_engine_dns()?;
        let server = servers.iter().find_map(|s| match s {
            IpAddr::V4(v4) => Some(*v4),
            IpAddr::V6(_) => None,
        })?;
        let (port, rx) = shared.register_pending(id);
        let pkt = packet::build_udp(shared.cfg.proxy_address, port, server, DNS_PORT, query);
        if !shared.forward_from_proxy(&pkt, engine, engine_ip) {
            shared.cancel_pending(port);
            return None;
        }
        let result = tokio::time::timeout(self.timeout, rx).await;
        shared.cancel_pending(port);
        result.ok()?.ok()
    }
}

impl Resolver for NetResolver {
    fn resolve<'a>(&'a self, via: Via, query: &'a [u8]) -> ResolveFuture<'a> {
        Box::pin(async move {
            match via {
                Via::Direct => self.direct(query).await,
                Via::Tunnel => self.tunnel(query).await,
            }
        })
    }
}
