//! Матчер правил: точные домены и `*.x`, подсети, исключения, `cidrs()`,
//! свои правила в `Storage`, `RulesChange` без пересоздания чего-либо.

use braid_core::RulesChange;
use braid_core::proto::{DomainPattern, Ipv4Net, Mode, RuleSet, RulesSpec};
use braid_core::rules::{
    EnabledSets, RuleBook, RulesError, RulesState, UserRules, normalize_domain,
};
use braid_core::testing::MemoryStorage;
use braid_core::{AppRules, Storage};
use std::net::Ipv4Addr;
use std::sync::Arc;

fn dp(s: &str) -> DomainPattern {
    DomainPattern::parse(s).unwrap()
}

fn net(s: &str) -> Ipv4Net {
    s.parse().unwrap()
}

/// Наборы как в фикстуре подписки: google (с исключениями) и cursor;
/// режим «work» включает оба и исключает calendar.google.com.
fn spec() -> RulesSpec {
    RulesSpec {
        sets: vec![
            RuleSet {
                id: "google".into(),
                name: "Google".into(),
                domains: vec![
                    dp("google.com"),
                    dp("*.google.com"),
                    dp("*.*.googleapis.com"),
                ],
                ips: vec![net("8.8.8.0/24"), net("142.250.0.0/15")],
                exclude_domains: vec![dp("mail.google.com")],
                exclude_ips: vec![net("142.251.1.0/24")],
            },
            RuleSet {
                id: "cursor".into(),
                name: "Cursor".into(),
                domains: vec![dp("cursor.com"), dp("*.cursor.com")],
                ips: vec![],
                exclude_domains: vec![],
                exclude_ips: vec![],
            },
            // Ни в одном режиме: включение даёт комбинацию «Своё».
            RuleSet {
                id: "notion".into(),
                name: "Notion".into(),
                domains: vec![dp("notion.so"), dp("*.notion.so")],
                ips: vec![],
                exclude_domains: vec![],
                exclude_ips: vec![],
            },
        ],
        modes: vec![
            Mode {
                id: "work".into(),
                name: "Работа".into(),
                sets: vec!["google".into(), "cursor".into()],
                exclude_domains: vec![dp("calendar.google.com")],
                exclude_ips: vec![],
                default: true,
            },
            Mode {
                id: "home".into(),
                name: "Дом".into(),
                sets: vec!["google".into()],
                exclude_domains: vec![],
                exclude_ips: vec![],
                default: false,
            },
        ],
    }
}

fn book_for(enabled: EnabledSets) -> RuleBook {
    RuleBook::from(&spec(), &UserRules::default(), &enabled)
}

#[test]
fn domains_exact_and_wildcard_table() {
    let book = book_for(EnabledSets::new(["google", "cursor"], Some("work".into())));
    let cases: &[(&str, bool, &str)] = &[
        ("google.com", true, "точное имя"),
        ("GOOGLE.COM.", true, "регистр и завершающая точка не важны"),
        ("www.google.com", true, "*.google.com"),
        ("a.b.google.com", true, "*.google.com на любой глубине"),
        ("notgoogle.com", false, "суффикс без точки — не поддомен"),
        (
            "googleapis.com",
            false,
            "*.*.googleapis.com не включает сам домен",
        ),
        (
            "a.googleapis.com",
            true,
            "*.*.googleapis.com = *.googleapis.com",
        ),
        ("a.b.googleapis.com", true, "многоуровневый шаблон Amnezia"),
        ("cursor.com", true, "второй набор включён"),
        ("api.cursor.com", true, "wildcard второго набора"),
        (
            "mail.google.com",
            false,
            "исключение набора побеждает *.google.com",
        ),
        (
            "x.mail.google.com",
            true,
            "исключение точное, поддомен не исключён",
        ),
        ("calendar.google.com", false, "исключение режима"),
        ("example.org", false, "не из правил"),
    ];
    for (name, want, why) in cases {
        assert_eq!(book.matches_domain(name), *want, "{name}: {why}");
    }
}

#[test]
fn disabled_set_and_custom_mode_drop_their_rules_and_mode_excludes() {
    let only_google = book_for(EnabledSets::new(["google"], None));
    assert!(only_google.matches_domain("google.com"));
    assert!(
        !only_google.matches_domain("api.cursor.com"),
        "набор выключен"
    );
    assert!(
        only_google.matches_domain("calendar.google.com"),
        "режим не выбран («Своё») — его исключения не действуют"
    );
    assert!(
        !only_google.matches_domain("mail.google.com"),
        "исключение набора действует всегда, пока набор включён"
    );

    let nothing = book_for(EnabledSets::new([] as [&str; 0], None));
    assert!(!nothing.matches_domain("google.com"));
    assert!(!nothing.matches_ip(Ipv4Addr::new(8, 8, 8, 8)));
    assert!(nothing.cidrs().is_empty());
}

#[test]
fn ips_and_cidrs_subtract_exclusions() {
    let book = book_for(EnabledSets::new(["google"], Some("work".into())));
    assert!(book.matches_ip(Ipv4Addr::new(8, 8, 8, 8)));
    assert!(book.matches_ip(Ipv4Addr::new(142, 250, 0, 1)));
    assert!(book.matches_ip(Ipv4Addr::new(142, 251, 2, 1)));
    assert!(
        !book.matches_ip(Ipv4Addr::new(142, 251, 1, 7)),
        "исключённая подсеть"
    );
    assert!(!book.matches_ip(Ipv4Addr::new(1, 1, 1, 1)));

    // 142.250.0.0/15 без 142.251.1.0/24 — разобрано вручную по CIDR.
    let want: Vec<Ipv4Net> = [
        "8.8.8.0/24",
        "142.250.0.0/16",
        "142.251.0.0/24",
        "142.251.2.0/23",
        "142.251.4.0/22",
        "142.251.8.0/21",
        "142.251.16.0/20",
        "142.251.32.0/19",
        "142.251.64.0/18",
        "142.251.128.0/17",
    ]
    .iter()
    .map(|s| net(s))
    .collect();
    assert_eq!(book.cidrs(), want);
}

#[test]
fn cidrs_drop_fully_excluded_and_aggregate_adjacent() {
    let spec = RulesSpec {
        sets: vec![RuleSet {
            id: "s".into(),
            name: "S".into(),
            domains: vec![],
            ips: vec![
                net("10.0.0.0/24"),
                net("10.0.1.0/24"),
                net("192.168.5.7/32"),
            ],
            exclude_domains: vec![],
            exclude_ips: vec![net("192.168.0.0/16")],
        }],
        modes: vec![],
    };
    let book = RuleBook::from(&spec, &UserRules::default(), &EnabledSets::new(["s"], None));
    assert_eq!(book.cidrs(), vec![net("10.0.0.0/23")]);
}

#[test]
fn user_rules_add_domain_with_subdomains_and_cidr_on_top() {
    let user = UserRules {
        domains: vec![dp("my.site"), dp("*.only-sub.net")],
        cidrs: vec![net("203.0.113.0/24")],
        apps: AppRules::All,
    };
    let book = RuleBook::from(&spec(), &user, &EnabledSets::new([] as [&str; 0], None));
    assert!(book.matches_domain("my.site"), "свой домен — сам");
    assert!(
        book.matches_domain("www.my.site"),
        "свой домен — и поддомены"
    );
    assert!(
        !book.matches_domain("only-sub.net"),
        "явный *. — только поддомены"
    );
    assert!(book.matches_domain("a.only-sub.net"));
    assert!(book.matches_ip(Ipv4Addr::new(203, 0, 113, 9)));
    assert_eq!(book.cidrs(), vec![net("203.0.113.0/24")]);
    assert!(!book.tunnel_all());

    let per_app = UserRules {
        apps: AppRules::Allowed(vec!["org.example.app".into()]),
        ..UserRules::default()
    };
    let book = RuleBook::from(&spec(), &per_app, &EnabledSets::new([] as [&str; 0], None));
    assert!(
        book.tunnel_all(),
        "режим «по приложениям»: весь трафик выбранных — в туннель"
    );
}

#[test]
fn normalize_domain_strips_url_parts_and_rejects_garbage() {
    assert_eq!(
        normalize_domain("https://My.Site/путь?x=1")
            .unwrap()
            .as_str(),
        "my.site"
    );
    assert_eq!(
        normalize_domain("  site.com:8443/ ").unwrap().as_str(),
        "site.com"
    );
    assert_eq!(
        normalize_domain("*.cdn.site.com").unwrap().as_str(),
        "*.cdn.site.com"
    );
    assert_eq!(
        normalize_domain("user@mail.site.com").unwrap().as_str(),
        "mail.site.com"
    );
    assert!(normalize_domain("not a domain").is_err());
    assert!(
        normalize_domain("localhost").is_err(),
        "одна метка — не домен"
    );
    assert!(
        normalize_domain("1.2.3.4").is_err(),
        "IP — это подсеть, не домен"
    );
    assert!(normalize_domain("").is_err());
}

#[test]
fn rules_state_applies_default_mode_and_changes_without_recreating_anything() {
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let mut state = RulesState::load(storage.clone(), spec()).unwrap();
    let view = state.view();
    assert_eq!(
        view.mode.as_deref(),
        Some("work"),
        "режим default включён сам (ист. 44)"
    );
    assert_eq!(
        view.sets
            .iter()
            .map(|s| (s.id.as_str(), s.enabled))
            .collect::<Vec<_>>(),
        vec![("google", true), ("cursor", true), ("notion", false)]
    );
    assert_eq!(view.sets[0].domains, 3);
    assert_eq!(view.sets[0].ips, 2);
    assert!(state.book().matches_domain("api.cursor.com"));
    assert!(!state.book().matches_domain("notion.so"));

    state
        .apply(RulesChange::ToggleSet {
            id: "notion".into(),
            on: true,
        })
        .unwrap();
    assert_eq!(
        state.view().mode,
        None,
        "своя комбинация — «Своё» (ист. 53)"
    );
    assert!(
        state.book().matches_domain("notion.so"),
        "включённый набор действует сразу"
    );
    assert!(
        state.book().matches_domain("calendar.google.com"),
        "без режима его исключения не действуют"
    );

    state
        .apply(RulesChange::ToggleSet {
            id: "notion".into(),
            on: false,
        })
        .unwrap();
    assert_eq!(
        state.view().mode.as_deref(),
        Some("work"),
        "пресет = положение переключателей: комбинация снова совпала с режимом"
    );
    assert!(
        !state.book().matches_domain("calendar.google.com"),
        "исключение режима вернулось"
    );

    state
        .apply(RulesChange::ToggleSet {
            id: "cursor".into(),
            on: false,
        })
        .unwrap();
    assert_eq!(
        state.view().mode.as_deref(),
        Some("home"),
        "{{google}} — это пресет «home»"
    );
    assert!(
        !state.book().matches_domain("api.cursor.com"),
        "выключенный набор → напрямую"
    );

    state
        .apply(RulesChange::SelectMode { id: "work".into() })
        .unwrap();
    assert_eq!(state.view().mode.as_deref(), Some("work"));
    assert!(state.book().matches_domain("cursor.com"));
    state
        .apply(RulesChange::SelectMode { id: "home".into() })
        .unwrap();
    assert_eq!(state.view().mode.as_deref(), Some("home"));
    assert!(!state.book().matches_domain("cursor.com"));
    assert!(
        state.book().matches_domain("calendar.google.com"),
        "у «home» нет исключений"
    );

    assert_eq!(
        state.apply(RulesChange::SelectMode { id: "nope".into() }),
        Err(RulesError::UnknownMode("nope".into()))
    );
    assert_eq!(
        state.apply(RulesChange::ToggleSet {
            id: "nope".into(),
            on: true
        }),
        Err(RulesError::UnknownSet("nope".into()))
    );

    let reloaded = RulesState::load(storage, spec()).unwrap();
    assert_eq!(
        reloaded.view().mode.as_deref(),
        Some("home"),
        "выбор режима пережил перезапуск"
    );
}

#[test]
fn user_rules_persist_and_survive_subscription_updates() {
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let mut state = RulesState::load(storage.clone(), spec()).unwrap();

    state
        .apply(RulesChange::AddUserDomain {
            domain: "https://My.Site/path".into(),
        })
        .unwrap();
    state
        .apply(RulesChange::AddUserCidr {
            cidr: net("203.0.113.0/24"),
        })
        .unwrap();
    state
        .apply(RulesChange::SetApps {
            apps: AppRules::Excluded(vec!["com.bank.app".into()]),
        })
        .unwrap();
    assert_eq!(
        state.apply(RulesChange::AddUserDomain {
            domain: "not a domain".into()
        }),
        Err(RulesError::Domain(braid_core::proto::DomainError::Invalid(
            "not a domain".into()
        )))
    );
    let view = state.view();
    assert_eq!(view.user_domains, vec!["my.site".to_string()]);
    assert_eq!(view.user_cidrs, vec![net("203.0.113.0/24")]);
    assert_eq!(view.apps, AppRules::Excluded(vec!["com.bank.app".into()]));
    assert!(state.book().matches_domain("www.my.site"));

    let stored = storage
        .read("user_rules.json")
        .unwrap()
        .expect("свои правила записаны в Storage");
    assert!(std::str::from_utf8(&stored).unwrap().contains("my.site"));

    // Обновление подписки: набор cursor удалён на сервере, google переименован.
    let mut updated = spec();
    updated.sets.truncate(1);
    updated.sets[0].name = "Google v2".into();
    updated.modes[0].sets = vec!["google".into()];
    let mut state = RulesState::load(storage.clone(), spec()).unwrap();
    state.set_spec(updated).unwrap();
    let view = state.view();
    assert_eq!(
        view.sets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["google"],
        "удалённый на сервере набор исчез (ист. 45)"
    );
    assert_eq!(view.sets[0].name, "Google v2", "одноимённый набор заменён");
    assert_eq!(
        view.user_domains,
        vec!["my.site".to_string()],
        "свои правила на месте"
    );
    assert_eq!(view.user_cidrs, vec![net("203.0.113.0/24")]);
    assert!(state.book().matches_domain("www.my.site"));
    assert!(!state.book().matches_domain("cursor.com"));
    assert_eq!(
        view.mode.as_deref(),
        Some("work"),
        "режим по id пережил обновление"
    );

    state
        .apply(RulesChange::RemoveUserDomain {
            domain: "my.site".into(),
        })
        .unwrap();
    state
        .apply(RulesChange::RemoveUserCidr {
            cidr: net("203.0.113.0/24"),
        })
        .unwrap();
    assert!(state.view().user_domains.is_empty());
    assert!(!state.book().matches_domain("www.my.site"));
    let reloaded = RulesState::load(storage, spec()).unwrap();
    assert!(
        reloaded.view().user_domains.is_empty(),
        "удаление тоже записано"
    );
}

#[test]
fn custom_toggles_keep_surviving_sets_when_subscription_changes() {
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    let mut state = RulesState::load(storage.clone(), spec()).unwrap();
    state
        .apply(RulesChange::ToggleSet {
            id: "google".into(),
            on: false,
        })
        .unwrap();
    assert_eq!(state.view().mode, None);

    let mut updated = spec();
    updated.sets.push(RuleSet {
        id: "twitch".into(),
        name: "Twitch".into(),
        domains: vec![dp("twitch.tv")],
        ips: vec![],
        exclude_domains: vec![],
        exclude_ips: vec![],
    });
    state.set_spec(updated).unwrap();
    let view = state.view();
    assert_eq!(
        view.sets
            .iter()
            .map(|s| (s.id.as_str(), s.enabled))
            .collect::<Vec<_>>(),
        vec![
            ("google", false),
            ("cursor", true),
            ("notion", false),
            ("twitch", false)
        ],
        "«Своё» сохраняет переключатели; новый набор выключен"
    );
    state.apply(RulesChange::FromSubscription).unwrap();
    assert_eq!(state.view().mode, None);
}
