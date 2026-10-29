//! Монитор живости (ист. 10, 25): DNS-проба случайного имени через движок
//! каждые 5 с (30 с при погашенном экране), окно 30 с; `Dead` после 3 проб
//! подряд без ответа или хендшейка AWG старше 180 с; `Degraded` при потерях
//! больше 50 %, медианном RTT больше 2 с или «стойле» (tx растёт, rx стоит
//! 15 с). Пробы уходят через подделку транспорта, ответы играет тест.

use braid_core::health::{
    DeadReason, DegradedReason, HealthConfig, HealthMonitor, Liveness, ProbeTransport,
};
use braid_core::proto::EngineId;
use braid_core::testing::FakeClock;
use braid_core::{Clock, EngineSnapshot, EngineState, Health};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
struct Sent {
    engine: EngineId,
    token: u16,
    query: Vec<u8>,
}

#[derive(Default)]
struct FakeTransport {
    sent: Mutex<Vec<Sent>>,
    cancelled: Mutex<Vec<u16>>,
    next_token: AtomicU16,
    refuse: AtomicBool,
}

impl FakeTransport {
    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }

    fn cancelled(&self) -> Vec<u16> {
        self.cancelled.lock().unwrap().clone()
    }
}

impl ProbeTransport for FakeTransport {
    fn send_probe(&self, engine: &EngineId, query: &[u8]) -> Option<u16> {
        if self.refuse.load(Ordering::SeqCst) {
            return None;
        }
        let token = self.next_token.fetch_add(1, Ordering::SeqCst) + 1;
        self.sent.lock().unwrap().push(Sent {
            engine: engine.clone(),
            token,
            query: query.to_vec(),
        });
        Some(token)
    }

    fn cancel_probe(&self, _engine: &EngineId, token: u16) {
        self.cancelled.lock().unwrap().push(token);
    }
}

struct Bench {
    clock: Arc<FakeClock>,
    transport: Arc<FakeTransport>,
    monitor: HealthMonitor,
    id: EngineId,
}

impl Bench {
    fn new() -> Self {
        let clock = Arc::new(FakeClock::new());
        let transport = Arc::new(FakeTransport::default());
        let monitor = HealthMonitor::new(clock.clone(), HealthConfig::default(), transport.clone());
        let id = EngineId::from("awg");
        monitor.track(&id);
        Self {
            clock,
            transport,
            monitor,
            id,
        }
    }

    fn tick(&self) {
        self.monitor.tick(self.clock.now());
    }

    fn advance(&self, secs: u64) {
        self.clock.advance(Duration::from_secs(secs));
    }

    /// Ответить на последнюю отправленную пробу: ответ несёт тот же
    /// идентификатор транзакции, что и запрос.
    fn reply_last(&self) -> bool {
        let last = self.transport.sent().pop().expect("проба отправлена");
        let mut reply = last.query.clone();
        reply[2] |= 0x80;
        self.monitor
            .on_reply(&last.engine, &reply, self.clock.now())
    }

    /// Проиграть последовательность: `R` — проба с ответом сразу, `L` — без
    /// ответа; между пробами 5 с.
    fn play(&self, script: &str) {
        for step in script.chars() {
            self.tick();
            if step == 'R' {
                assert!(self.reply_last(), "ответ на отправленную пробу");
            }
            self.advance(5);
        }
        self.tick();
    }

    fn liveness(&self) -> Liveness {
        self.monitor.liveness(&self.id)
    }

    fn observe(&self, snapshot: EngineSnapshot) {
        self.monitor.observe(&self.id, &snapshot, self.clock.now());
    }
}

fn connected() -> EngineSnapshot {
    EngineSnapshot {
        state: EngineState::Connected,
        ..EngineSnapshot::default()
    }
}

#[test]
fn probes_go_every_5s_awake_and_every_30s_when_screen_is_off() {
    let b = Bench::new();
    b.tick();
    assert_eq!(b.transport.sent().len(), 1, "первая проба — сразу");
    for _ in 0..4 {
        b.advance(1);
        b.tick();
    }
    assert_eq!(b.transport.sent().len(), 1, "раньше 5 с второй пробы нет");
    b.advance(1);
    b.tick();
    let sent = b.transport.sent();
    assert_eq!(sent.len(), 2);
    assert_ne!(
        sent[0].query[..2],
        sent[1].query[..2],
        "разные идентификаторы"
    );
    assert_ne!(
        sent[0].query[12..],
        sent[1].query[12..],
        "разные случайные имена"
    );
    assert_eq!(sent[1].engine, b.id);
    assert_eq!(
        sent[1].query[2..4],
        [0x01, 0x00],
        "обычный запрос с RD, без ответа"
    );

    // Экран погашен — следующая проба через 30 с от предыдущей, не через 5.
    b.monitor.set_idle(true);
    b.advance(5);
    b.tick();
    assert_eq!(b.transport.sent().len(), 2);
    b.advance(24);
    b.tick();
    assert_eq!(b.transport.sent().len(), 2);
    b.advance(1);
    b.tick();
    assert_eq!(b.transport.sent().len(), 3, "через 30 с — проба");
}

#[test]
fn loss_table_alive_degraded_dead() {
    let table = [
        ("RRR", Liveness::Alive),
        ("RL", Liveness::Alive),
        ("LL", Liveness::Unknown),
        ("RLL", Liveness::Degraded(DegradedReason::Loss)),
        ("LLLR", Liveness::Degraded(DegradedReason::Loss)),
        ("LLL", Liveness::Dead(DeadReason::NoReply)),
        ("RLLL", Liveness::Dead(DeadReason::NoReply)),
        // Три потери были 30 с назад и вышли из окна.
        ("LLLRRRRRRR", Liveness::Alive),
    ];
    for (script, expected) in table {
        let b = Bench::new();
        b.observe(connected());
        b.play(script);
        assert_eq!(b.liveness(), expected, "сценарий {script}");
    }
}

#[test]
fn unanswered_probe_is_cancelled_after_timeout() {
    let b = Bench::new();
    b.tick();
    let token = b.transport.sent()[0].token;
    b.advance(3);
    b.tick();
    assert!(b.transport.cancelled().is_empty(), "4 с ещё не прошло");
    b.advance(1);
    let outcomes = b.monitor.tick(b.clock.now());
    assert_eq!(b.transport.cancelled(), vec![token]);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        (outcomes[0].engine.clone(), outcomes[0].rtt),
        (b.id.clone(), None)
    );
}

#[test]
fn handshake_older_than_180s_is_dead_even_with_replies() {
    for (age, expected) in [
        (180, Liveness::Alive),
        (181, Liveness::Dead(DeadReason::HandshakeStale)),
    ] {
        let b = Bench::new();
        b.play("RRR");
        b.observe(EngineSnapshot {
            handshake_age: Some(Duration::from_secs(age)),
            ..connected()
        });
        assert_eq!(b.liveness(), expected, "хендшейк {age} с");
    }
}

#[test]
fn median_rtt_above_2s_degrades() {
    for (rtts_ms, expected) in [
        (
            [100u64, 2500, 2500],
            Liveness::Degraded(DegradedReason::Rtt),
        ),
        ([100, 200, 2500], Liveness::Alive),
    ] {
        let b = Bench::new();
        b.observe(connected());
        for rtt in rtts_ms {
            b.tick();
            b.clock.advance(Duration::from_millis(rtt));
            assert!(b.reply_last());
            b.clock.advance(Duration::from_millis(5000 - rtt));
        }
        b.tick();
        assert_eq!(b.liveness(), expected, "RTT {rtts_ms:?}");
    }
}

#[test]
fn stall_tx_grows_rx_flat_for_15s_degrades() {
    let b = Bench::new();
    let snap = |tx, rx| EngineSnapshot {
        tx_bytes: tx,
        rx_bytes: rx,
        ..connected()
    };
    b.observe(snap(100, 50));
    b.play("R");
    for (tx, rx) in [(200, 50), (300, 50)] {
        b.observe(snap(tx, rx));
        b.play("R");
    }
    assert_eq!(b.liveness(), Liveness::Alive, "10 с стойла — ещё не порог");
    b.observe(snap(400, 50));
    assert_eq!(
        b.liveness(),
        Liveness::Degraded(DegradedReason::Stall),
        "15 с tx растёт, rx стоит"
    );
    b.play("R");
    b.observe(snap(500, 60));
    assert_eq!(b.liveness(), Liveness::Alive, "rx пошёл — стойла нет");
}

#[test]
fn waiting_for_user_is_not_death() {
    let b = Bench::new();
    b.observe(EngineSnapshot {
        state: EngineState::WaitingUser,
        ..EngineSnapshot::default()
    });
    b.play("LLL");
    assert_eq!(b.liveness(), Liveness::WaitingUser);
}

#[test]
fn report_has_median_rtt_loss_and_handshake_age() {
    let b = Bench::new();
    b.observe(EngineSnapshot {
        handshake_age: Some(Duration::from_secs(42)),
        ..connected()
    });
    for rtt in [100u64, 500, 300] {
        b.tick();
        b.clock.advance(Duration::from_millis(rtt));
        assert!(b.reply_last());
        b.clock.advance(Duration::from_millis(5000 - rtt));
    }
    b.play("L");
    assert_eq!(
        b.monitor.report(&b.id),
        Health {
            rtt_ms: Some(300),
            loss_pct: 25,
            handshake_age_s: Some(42),
        }
    );
}

#[test]
fn refused_probe_is_not_counted_as_loss() {
    let b = Bench::new();
    b.transport.refuse.store(true, Ordering::SeqCst);
    b.play("LLL");
    assert_eq!(
        b.liveness(),
        Liveness::Unknown,
        "движок ещё без адреса — пробы не ушли"
    );
    assert_eq!(b.monitor.report(&b.id).loss_pct, 0);
}
