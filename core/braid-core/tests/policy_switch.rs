//! Политика автовыбора (ист. 22–24, 26, 30, 31) — чистый автомат на
//! `FakeClock`: приоритет из подписки, тёплый запас — следующий по
//! приоритету, смерть активного → тёплый сразу, деградация → только на
//! здоровый, возврат на приоритетный после 60 с стабильных проб, не чаще
//! одного переключения в 2 мин (кроме смерти), закрепление отключает
//! автоматику. Память сети (ист. 23): после 3 неудач подряд старт в этой
//! сети с другого движка; переживает перезапуск через `Storage`.

use braid_core::health::{DeadReason, DegradedReason, Liveness};
use braid_core::policy::{Action, NetworkMemory, Outcome, Policy, PolicyConfig, PolicyInputs};
use braid_core::proto::{EngineId, NetworkId, SwitchReason};
use braid_core::testing::{FakeClock, MemoryStorage};
use braid_core::{Clock, EngineMode};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

fn id(s: &str) -> EngineId {
    EngineId::from(s)
}

fn priority() -> Vec<EngineId> {
    vec![id("awg"), id("csqtt")]
}

fn liveness(pairs: &[(&str, Liveness)]) -> BTreeMap<EngineId, Liveness> {
    pairs.iter().map(|(e, l)| (id(e), *l)).collect()
}

struct Bench {
    clock: Arc<FakeClock>,
    policy: Policy,
    priority: Vec<EngineId>,
    mode: EngineMode,
}

impl Bench {
    fn new() -> Self {
        Self {
            clock: Arc::new(FakeClock::new()),
            policy: Policy::new(PolicyConfig::default()),
            priority: priority(),
            mode: EngineMode::Auto,
        }
    }

    fn decide(&mut self, active: &str, health: &[(&str, Liveness)]) -> Action {
        let map = liveness(health);
        let inputs = PolicyInputs {
            priority: &self.priority,
            liveness: &map,
            active: Some(&id(active)),
            mode: &self.mode,
        };
        self.policy.decide(&inputs, self.clock.now()).action
    }

    fn warm(&mut self, active: &str, health: &[(&str, Liveness)]) -> Option<EngineId> {
        let map = liveness(health);
        let inputs = PolicyInputs {
            priority: &self.priority,
            liveness: &map,
            active: Some(&id(active)),
            mode: &self.mode,
        };
        self.policy.decide(&inputs, self.clock.now()).warm
    }

    fn advance(&self, secs: u64) {
        self.clock.advance(Duration::from_secs(secs));
    }
}

fn switch(to: &str, reason: SwitchReason) -> Action {
    Action::Switch { to: id(to), reason }
}

const ALIVE: Liveness = Liveness::Alive;
const DEAD: Liveness = Liveness::Dead(DeadReason::NoReply);

#[test]
fn startup_takes_priority_engine_and_warms_the_next() {
    let mut b = Bench::new();
    assert_eq!(
        Policy::initial(&b.priority, None, &EngineMode::Auto),
        Some((id("awg"), SwitchReason::Startup))
    );
    assert_eq!(
        b.decide("awg", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Action::Stay
    );
    assert_eq!(
        b.warm("awg", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Some(id("csqtt"))
    );
    assert_eq!(
        b.warm("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Some(id("awg")),
        "тёплый — следующий по приоритету после активного"
    );
    assert_eq!(
        Policy::initial(&b.priority, None, &EngineMode::Pinned(id("csqtt"))),
        Some((id("csqtt"), SwitchReason::Manual))
    );
}

#[test]
fn dead_active_switches_to_warm_at_once_even_right_after_a_switch() {
    let mut b = Bench::new();
    assert_eq!(
        b.decide("awg", &[("awg", DEAD), ("csqtt", ALIVE)]),
        switch("csqtt", SwitchReason::Dead)
    );
    // Только что переключились — но смерть не ждёт двух минут.
    b.advance(10);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", DEAD)]),
        switch("awg", SwitchReason::Dead)
    );
    // Смерть по хендшейку и по сообщению движка — тот же код.
    for reason in [DeadReason::HandshakeStale, DeadReason::Engine] {
        let mut b = Bench::new();
        assert_eq!(
            b.decide("awg", &[("awg", Liveness::Dead(reason)), ("csqtt", ALIVE)]),
            switch("csqtt", SwitchReason::Dead),
            "{reason:?}"
        );
    }
}

#[test]
fn degraded_active_switches_only_to_a_healthy_warm_engine() {
    let table = [
        (
            DegradedReason::Loss,
            ALIVE,
            switch("csqtt", SwitchReason::Degraded),
        ),
        (
            DegradedReason::Rtt,
            ALIVE,
            switch("csqtt", SwitchReason::Degraded),
        ),
        (
            DegradedReason::Stall,
            ALIVE,
            switch("csqtt", SwitchReason::Stall),
        ),
        (
            DegradedReason::Loss,
            Liveness::Degraded(DegradedReason::Loss),
            Action::Stay,
        ),
        (DegradedReason::Loss, Liveness::Unknown, Action::Stay),
        (DegradedReason::Loss, DEAD, Action::Stay),
    ];
    for (reason, warm, expected) in table {
        let mut b = Bench::new();
        assert_eq!(
            b.decide(
                "awg",
                &[("awg", Liveness::Degraded(reason)), ("csqtt", warm)]
            ),
            expected,
            "{reason:?} при тёплом {warm:?}"
        );
    }
}

#[test]
fn returns_to_priority_after_60s_of_stable_probes() {
    let mut b = Bench::new();
    assert_eq!(
        b.decide("awg", &[("awg", DEAD), ("csqtt", ALIVE)]),
        switch("csqtt", SwitchReason::Dead)
    );
    b.advance(120);
    // AWG ожил только что: стабильности ещё нет.
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Action::Stay
    );
    b.advance(59);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Action::Stay
    );
    b.advance(1);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        switch("awg", SwitchReason::Priority)
    );
}

#[test]
fn stability_counter_restarts_when_priority_engine_flaps() {
    let mut b = Bench::new();
    b.decide("awg", &[("awg", DEAD), ("csqtt", ALIVE)]);
    b.advance(120);
    b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]);
    b.advance(40);
    b.decide(
        "csqtt",
        &[
            ("awg", Liveness::Degraded(DegradedReason::Loss)),
            ("csqtt", ALIVE),
        ],
    );
    b.advance(1);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Action::Stay
    );
    b.advance(29);
    // 70 с с первого оживания, но 30 с после провала — ещё не стабильно.
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Action::Stay
    );
    b.advance(31);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        switch("awg", SwitchReason::Priority)
    );
}

#[test]
fn at_most_one_switch_per_two_minutes_except_death() {
    let mut b = Bench::new();
    assert_eq!(
        b.decide(
            "awg",
            &[
                ("awg", Liveness::Degraded(DegradedReason::Loss)),
                ("csqtt", ALIVE)
            ]
        ),
        switch("csqtt", SwitchReason::Degraded)
    );
    // AWG ожил через 10 с и стабилен минуту, но с переключения прошло лишь 70 с.
    b.advance(10);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Action::Stay
    );
    b.advance(60);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Action::Stay
    );
    b.advance(49);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        Action::Stay
    );
    b.advance(1);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        switch("awg", SwitchReason::Priority)
    );
    // Деградация тоже ждёт лимита, смерть — нет.
    b.advance(30);
    assert_eq!(
        b.decide(
            "awg",
            &[
                ("awg", Liveness::Degraded(DegradedReason::Rtt)),
                ("csqtt", ALIVE)
            ]
        ),
        Action::Stay
    );
    assert_eq!(
        b.decide("awg", &[("awg", DEAD), ("csqtt", ALIVE)]),
        switch("csqtt", SwitchReason::Dead)
    );
}

#[test]
fn pinned_engine_disables_automation_and_is_not_replaced_when_dead() {
    let mut b = Bench::new();
    b.mode = EngineMode::Pinned(id("awg"));
    assert_eq!(
        b.decide("awg", &[("awg", DEAD), ("csqtt", ALIVE)]),
        Action::Stay
    );
    assert_eq!(
        b.warm("awg", &[("awg", DEAD), ("csqtt", ALIVE)]),
        Some(id("csqtt")),
        "тёплый держится и при закреплении"
    );
    // Закрепили другой — ручное переключение сразу, без ожидания лимитов.
    b.mode = EngineMode::Pinned(id("csqtt"));
    assert_eq!(
        b.decide("awg", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        switch("csqtt", SwitchReason::Manual)
    );
    b.advance(1);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", DEAD)]),
        Action::Stay
    );
    // Назад в «Авто»: приоритетный жив давно — возврат по приоритету.
    b.mode = EngineMode::Auto;
    b.advance(120);
    assert_eq!(
        b.decide("csqtt", &[("awg", ALIVE), ("csqtt", ALIVE)]),
        switch("awg", SwitchReason::Priority)
    );
}

#[test]
fn starting_active_gets_time_before_any_switch() {
    let mut b = Bench::new();
    assert_eq!(
        b.decide("awg", &[("awg", Liveness::Unknown), ("csqtt", ALIVE)]),
        Action::Stay,
        "первых ответов ещё не было — не смерть"
    );
    assert_eq!(
        b.warm("awg", &[("awg", Liveness::Unknown), ("csqtt", ALIVE)]),
        Some(id("csqtt"))
    );
}

#[test]
fn everything_dead_means_none_alive_until_someone_returns() {
    let mut b = Bench::new();
    assert_eq!(
        b.decide("awg", &[("awg", DEAD), ("csqtt", DEAD)]),
        Action::NoneAlive
    );
    assert_eq!(
        b.decide("awg", &[("awg", DEAD), ("csqtt", Liveness::WaitingUser)]),
        Action::NoneAlive,
        "ждущий пользователя трафик не понесёт"
    );
    b.advance(5);
    assert_eq!(
        b.decide("awg", &[("awg", DEAD), ("csqtt", ALIVE)]),
        switch("csqtt", SwitchReason::Dead)
    );
}

#[test]
fn network_memory_prefers_last_working_engine_and_skips_three_failures() {
    let storage = Arc::new(MemoryStorage::new());
    let net = NetworkId::from("wifi:home");
    let other = NetworkId::from("lte:op");
    let mut memory = NetworkMemory::load(storage.clone());
    assert_eq!(memory.preferred(&net), None);
    assert_eq!(
        Policy::initial(&priority(), Some((&memory, &net)), &EngineMode::Auto),
        Some((id("awg"), SwitchReason::Startup))
    );

    // Здесь работал CSQTT — в известной сети старт с него.
    memory.record(&net, &id("csqtt"), Outcome::Worked);
    assert_eq!(memory.preferred(&net), Some(id("csqtt")));
    assert_eq!(
        Policy::initial(&priority(), Some((&memory, &net)), &EngineMode::Auto),
        Some((id("csqtt"), SwitchReason::Memory))
    );
    assert_eq!(memory.preferred(&other), None, "другая сеть — своя память");

    // Две неудачи AWG подряд — ещё не приговор, третья — старт с другого.
    memory.record(&net, &id("awg"), Outcome::Worked);
    memory.record(&net, &id("awg"), Outcome::Failed);
    memory.record(&net, &id("awg"), Outcome::Failed);
    assert_eq!(memory.failures(&net, &id("awg")), 2);
    assert_eq!(
        Policy::initial(&priority(), Some((&memory, &net)), &EngineMode::Auto),
        Some((id("awg"), SwitchReason::Memory))
    );
    memory.record(&net, &id("awg"), Outcome::Failed);
    assert_eq!(memory.failures(&net, &id("awg")), 3);
    assert_eq!(
        Policy::initial(&priority(), Some((&memory, &net)), &EngineMode::Auto),
        Some((id("csqtt"), SwitchReason::Memory)),
        "три неудачи подряд — старт с CSQTT"
    );
    // Успех обнуляет серию.
    memory.record(&net, &id("awg"), Outcome::Worked);
    assert_eq!(memory.failures(&net, &id("awg")), 0);

    // Переживает перезапуск.
    memory.record(&net, &id("awg"), Outcome::Failed);
    let reloaded = NetworkMemory::load(storage);
    assert_eq!(reloaded.failures(&net, &id("awg")), 1);
    assert_eq!(reloaded.preferred(&net), Some(id("awg")));
    assert_eq!(reloaded.preferred(&other), None);
}
