//! DNS-прокси и fake-IP — тикет 02.
//!
//! [`DnsProxy::handle`]: A-запрос домена из правил → адрес из `198.18.0.0/15`
//! с TTL 60 с, настоящий адрес резолвится через туннель заранее и хранится
//! в [`FakeIpPool`]; AAAA (и прочие типы) того же домена → NODATA; домен не
//! из правил → исходные резолверы системы или, в `DnsMode::AllViaTunnel`,
//! резолвер движка — ответ отдаётся как есть. Kill switch: без живых движков
//! домен из правил → SERVFAIL, прямой DNS продолжает работать.
//! Запись пула живёт 24 ч без обращений, потом адрес переиспользуется.
//! [`TcpDns`] принимает те же запросы по TCP/53.

pub mod message;
mod pool;
mod proxy;
mod tcp;

pub use pool::{FAKE_IDLE, FAKE_TTL, FakeEntry, FakeIpPool, fake_range};
pub use proxy::{DnsCtx, DnsProxy, ResolveFuture, Resolver, StaticResolver, Via};
pub use tcp::{ConnKey, TcpDns, TcpStep};
