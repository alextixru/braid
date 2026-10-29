//! DNS-прокси и fake-IP — тикет 02.
//!
//! `DnsProxy::handle(query, ctx) -> Reply`: A-запрос домена из правил → адрес из
//! `198.18.0.0/15` с TTL 60 с; AAAA того же домена → NODATA; домен не из правил →
//! исходные резолверы системы (`Platform::original_dns`) или, в `DnsMode::AllViaTunnel`,
//! резолвер движка. `FakeIpPool { alloc(name), lookup(ip), gc() }`: запись живёт
//! 24 ч без обращения; настоящий адрес резолвится через туннель заранее.
//! Kill switch: без живых движков домен из правил → SERVFAIL.
