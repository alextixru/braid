//! Политика автовыбора — тикет 03 (ист. 22–24, 26, 30, 31). Чистый автомат
//! без ввода-вывода: вход — приоритет из подписки, вердикты монитора
//! живости, активный движок и режим; выход — [`Decision`] с кодом причины
//! ([`SwitchReason`]) для журнала.
//!
//! - активный несёт трафик, тёплый запас — следующий по приоритету;
//! - активный мёртв (или ждёт пользователя) → на любой пригодный, сразу,
//!   лимиты не действуют; пригодных нет → [`Action::NoneAlive`];
//! - активный деградировал → только на здоровый (`Alive`) запас;
//! - возврат на приоритетный — после [`PolicyConfig::return_after`] (60 с)
//!   непрерывно `Alive`; любое переключение, кроме смерти и ручного, —
//!   не чаще [`PolicyConfig::min_switch_gap`] (2 мин);
//! - `Pinned` отключает автоматику: мёртвый закреплённый не подменяется.
//!
//! [`NetworkMemory`] (ист. 23): по `NetworkId` — последний рабочий движок и
//! неудачи подряд; после [`FAILURES_BEFORE_SKIP`] неудач старт в этой сети
//! начинается с другого движка; живёт в [`Storage`] под [`MEMORY_KEY`].

use crate::health::Liveness;
use crate::state::EngineMode;
use crate::storage::Storage;
use braid_proto::{EngineId, NetworkId, SwitchReason};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyConfig {
    /// Столько непрерывно живых проб приоритетного — и возвращаемся на него.
    pub return_after: Duration,
    /// Минимум между переключениями (кроме смерти и ручных).
    pub min_switch_gap: Duration,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            return_after: Duration::from_secs(60),
            min_switch_gap: Duration::from_secs(120),
        }
    }
}

/// Вход автомата на один шаг.
pub struct PolicyInputs<'a> {
    /// Движки в порядке приоритета из подписки.
    pub priority: &'a [EngineId],
    pub liveness: &'a BTreeMap<EngineId, Liveness>,
    pub active: Option<&'a EngineId>,
    pub mode: &'a EngineMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Stay,
    Switch {
        to: EngineId,
        reason: SwitchReason,
    },
    /// Нести трафик некому — kill switch и «Нет связи с хостом».
    NoneAlive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub action: Action,
    /// Кого держать тёплым после этого шага.
    pub warm: Option<EngineId>,
}

pub struct Policy {
    cfg: PolicyConfig,
    last_switch: Option<Instant>,
    /// С какого момента движок непрерывно `Alive`.
    alive_since: BTreeMap<EngineId, Instant>,
}

impl Default for Policy {
    fn default() -> Self {
        Self::new(PolicyConfig::default())
    }
}

impl Policy {
    pub fn new(cfg: PolicyConfig) -> Self {
        Self {
            cfg,
            last_switch: None,
            alive_since: BTreeMap::new(),
        }
    }

    pub fn config(&self) -> &PolicyConfig {
        &self.cfg
    }

    pub fn last_switch(&self) -> Option<Instant> {
        self.last_switch
    }

    /// Переключение случилось вне автомата (ручное) — лимиты считаются от него.
    pub fn note_switch(&mut self, now: Instant) {
        self.last_switch = Some(now);
    }

    /// Новая сессия: история стабильности и переключений забыта.
    pub fn reset(&mut self) {
        self.last_switch = None;
        self.alive_since.clear();
    }

    /// С какого движка начинать сессию: закреплённый; в известной сети —
    /// последний рабочий, а движки с тремя неудачами подряд пропускаются;
    /// иначе первый по приоритету. `None` — движков нет.
    pub fn initial(
        priority: &[EngineId],
        memory: Option<(&NetworkMemory, &NetworkId)>,
        mode: &EngineMode,
    ) -> Option<(EngineId, SwitchReason)> {
        if let EngineMode::Pinned(id) = mode
            && priority.contains(id)
        {
            return Some((id.clone(), SwitchReason::Manual));
        }
        let first = priority.first()?;
        if let Some((memory, net)) = memory {
            if let Some(preferred) = memory.preferred(net)
                && priority.contains(&preferred)
            {
                return Some((preferred, SwitchReason::Memory));
            }
            if let Some(candidate) = priority.iter().find(|id| !memory.is_blocked(net, id))
                && candidate != first
            {
                return Some((candidate.clone(), SwitchReason::Memory));
            }
        }
        Some((first.clone(), SwitchReason::Startup))
    }

    /// Тёплый запас — первый по приоритету, кроме активного.
    pub fn warm_for(priority: &[EngineId], active: Option<&EngineId>) -> Option<EngineId> {
        priority.iter().find(|id| Some(*id) != active).cloned()
    }

    pub fn decide(&mut self, inputs: &PolicyInputs, now: Instant) -> Decision {
        let liveness = |id: &EngineId| {
            inputs
                .liveness
                .get(id)
                .copied()
                .unwrap_or(Liveness::Unknown)
        };
        for id in inputs.priority {
            if liveness(id) == Liveness::Alive {
                self.alive_since.entry(id.clone()).or_insert(now);
            } else {
                self.alive_since.remove(id);
            }
        }
        let gap_ok = self
            .last_switch
            .is_none_or(|t| now.duration_since(t) >= self.cfg.min_switch_gap);
        let action = match inputs.mode {
            EngineMode::Pinned(pinned) => {
                if inputs.active != Some(pinned) && inputs.priority.contains(pinned) {
                    Action::Switch {
                        to: pinned.clone(),
                        reason: SwitchReason::Manual,
                    }
                } else {
                    Action::Stay
                }
            }
            EngineMode::Auto => match inputs.active {
                None => match inputs.priority.iter().find(|id| liveness(id).is_usable()) {
                    Some(to) => Action::Switch {
                        to: to.clone(),
                        reason: SwitchReason::Startup,
                    },
                    None => Action::NoneAlive,
                },
                Some(active) => {
                    let current = liveness(active);
                    if matches!(current, Liveness::Dead(_) | Liveness::WaitingUser) {
                        // Смерть (или ожидание пользователя): на любой пригодный, сразу.
                        match inputs
                            .priority
                            .iter()
                            .find(|id| *id != active && liveness(id).is_usable())
                        {
                            Some(to) => Action::Switch {
                                to: to.clone(),
                                reason: current.switch_reason().unwrap_or(SwitchReason::Dead),
                            },
                            None => Action::NoneAlive,
                        }
                    } else if let Liveness::Degraded(_) = current {
                        match inputs
                            .priority
                            .iter()
                            .find(|id| *id != active && liveness(id) == Liveness::Alive)
                        {
                            Some(to) if gap_ok => Action::Switch {
                                to: to.clone(),
                                reason: current.switch_reason().unwrap_or(SwitchReason::Degraded),
                            },
                            _ => Action::Stay,
                        }
                    } else if current == Liveness::Unknown {
                        // Активный ещё не ответил ни разу — ему дают время (ядро
                        // объявит смерть по истечении срока старта).
                        Action::Stay
                    } else {
                        // Активный жив: возврат на более приоритетный после гистерезиса.
                        let better =
                            inputs
                                .priority
                                .iter()
                                .take_while(|id| *id != active)
                                .find(|id| {
                                    self.alive_since.get(*id).is_some_and(|since| {
                                        now.duration_since(*since) >= self.cfg.return_after
                                    })
                                });
                        match better {
                            Some(to) if gap_ok => Action::Switch {
                                to: to.clone(),
                                reason: SwitchReason::Priority,
                            },
                            _ => Action::Stay,
                        }
                    }
                }
            },
        };
        let next_active = match &action {
            Action::Switch { to, .. } => {
                self.last_switch = Some(now);
                Some(to)
            }
            Action::Stay | Action::NoneAlive => inputs.active,
        };
        Decision {
            warm: Self::warm_for(inputs.priority, next_active),
            action,
        }
    }
}

/// Чем кончилась работа движка в сети.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Worked,
    Failed,
}

pub const MEMORY_KEY: &str = "network_memory.json";
/// Столько неудач подряд — и старт в этой сети начинается с другого движка.
pub const FAILURES_BEFORE_SKIP: u32 = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct NetworkRecord {
    #[serde(default)]
    last_worked: Option<EngineId>,
    /// Неудачи подряд по движкам; успех обнуляет.
    #[serde(default)]
    failures: BTreeMap<EngineId, u32>,
}

pub struct NetworkMemory {
    storage: Arc<dyn Storage>,
    nets: BTreeMap<NetworkId, NetworkRecord>,
}

impl NetworkMemory {
    /// Прочитать из хранилища; битая запись — пустая память и предупреждение.
    pub fn load(storage: Arc<dyn Storage>) -> Self {
        let nets = match storage.read(MEMORY_KEY) {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("память сети не читается, начинаю с пустой: {e}");
                BTreeMap::new()
            }),
            Ok(None) => BTreeMap::new(),
            Err(e) => {
                tracing::warn!("память сети не прочитана: {e}");
                BTreeMap::new()
            }
        };
        Self { storage, nets }
    }

    fn save(&self) {
        let bytes = serde_json::to_vec_pretty(&self.nets).expect("память сети сериализуется");
        if let Err(e) = self.storage.write(MEMORY_KEY, &bytes) {
            tracing::warn!("память сети не записана: {e}");
        }
    }

    pub fn record(&mut self, net: &NetworkId, engine: &EngineId, outcome: Outcome) {
        let record = self.nets.entry(net.clone()).or_default();
        match outcome {
            Outcome::Worked => {
                record.last_worked = Some(engine.clone());
                record.failures.insert(engine.clone(), 0);
            }
            Outcome::Failed => {
                let count = record.failures.entry(engine.clone()).or_insert(0);
                *count += 1;
                if *count >= FAILURES_BEFORE_SKIP && record.last_worked.as_ref() == Some(engine) {
                    record.last_worked = None;
                }
            }
        }
        self.save();
    }

    /// Последний рабочий движок сети, если он не провалился трижды подряд.
    pub fn preferred(&self, net: &NetworkId) -> Option<EngineId> {
        let record = self.nets.get(net)?;
        let engine = record.last_worked.clone()?;
        (!self.is_blocked(net, &engine)).then_some(engine)
    }

    pub fn failures(&self, net: &NetworkId, engine: &EngineId) -> u32 {
        self.nets
            .get(net)
            .and_then(|r| r.failures.get(engine))
            .copied()
            .unwrap_or(0)
    }

    pub fn is_blocked(&self, net: &NetworkId, engine: &EngineId) -> bool {
        self.failures(net, engine) >= FAILURES_BEFORE_SKIP
    }
}
