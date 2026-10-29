//! Матчер правил — тикет 02.
//!
//! `RuleBook { from(sub: &RulesSpec, user: &UserRules, enabled: &EnabledSets) -> Self,
//! matches_domain(&str) -> bool, matches_ip(Ipv4Addr) -> bool, cidrs() -> Vec<Ipv4Net> }`.
//! Точные домены и `*.x` (включая `*.*.x` из списков Amnezia); исключения
//! (`exclude_*` включённых наборов и режима) побеждают включения; `cidrs()` —
//! включающие подсети за вычетом исключённых. Свои правила пользователя
//! (`UserRules`) живут в `Storage` и переживают обновление наборов из подписки.
