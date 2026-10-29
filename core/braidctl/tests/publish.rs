//! `publish` и `verify`: подписки всех пользователей хоста, версия растёт только
//! при изменении содержимого, выкладка атомарная, сверка хешей с хостом.

mod common;

use braid_proto::Subscription;
use braidctl::State;
use common::{FakeHost, init_state, run_err, run_ok, write_host_helsinki};
use std::fs;

fn two_users(tag: &str) -> (State, FakeHost) {
    let state = init_state(tag);
    write_host_helsinki(&state);
    let fake = FakeHost::amnezia();
    run_ok(
        &state,
        &fake.shell,
        &["add-user", "alex", "--host", "helsinki", "--admin"],
    );
    run_ok(
        &state,
        &fake.shell,
        &["add-user", "bob", "--host", "helsinki"],
    );
    fake.shell.clear();
    (state, fake)
}

fn token(state: &State, name: &str) -> String {
    state
        .load_user_secrets("helsinki", name)
        .unwrap()
        .token
        .unwrap()
}

#[test]
fn publish_bumps_version_only_when_content_changes_and_uploads_atomically() {
    let (state, fake) = two_users("publish");

    // Ничего не менялось — ничего не выкладывается, версии те же.
    let out = run_ok(&state, &fake.shell, &["publish"]);
    assert_eq!(fake.shell.count("subd.put"), 0, "{:?}", fake.shell.steps());
    assert_eq!(fake.shell.count("subd.hashes"), 1, "одна сверка на хост");
    assert!(out.contains("без изменений"), "{out}");
    assert_eq!(state.load_user("alex").unwrap().version, 1);

    // Правка набора — у обоих версия 2, файлы на хосте обновлены, client.latest подхвачен.
    fs::write(
        state.rules_dir().join("cursor.json"),
        r#"{"id":"cursor","name":"Cursor","domains":["cursor.com","*.cursor.com","cursor.sh"],"ips":[]}"#,
    )
    .unwrap();
    fs::write(
        state.client_path(),
        "latest = \"0.2.0\"\n[downloads]\nmacos = \"https://example.invalid/Braid.zip\"\n",
    )
    .unwrap();
    fake.shell.clear();
    let out = run_ok(&state, &fake.shell, &["publish", "--host", "helsinki"]);
    assert_eq!(fake.shell.count("subd.put"), 2, "{:?}", fake.shell.steps());
    assert!(
        out.contains("alex") && out.contains("bob") && out.contains("v2"),
        "{out}"
    );
    for name in ["alex", "bob"] {
        let user = state.load_user(name).unwrap();
        assert_eq!(user.version, 2, "{name}");
        let body = fake
            .state
            .borrow()
            .subs
            .get(&token(&state, name))
            .cloned()
            .unwrap();
        let sub = Subscription::from_json(&body).unwrap();
        assert_eq!(sub.version, 2);
        assert!(
            sub.rules
                .sets
                .iter()
                .any(|s| s.domains.iter().any(|d| d.as_str() == "cursor.sh"))
        );
        assert_eq!(sub.client.as_ref().unwrap().latest, "0.2.0");
        assert_eq!(sub.user.admin, name == "alex");
        assert_eq!(
            sub.telemetry.is_some(),
            name == "alex",
            "телеметрия только у админа"
        );
        assert_eq!(
            state.load_published("helsinki", name).unwrap().unwrap(),
            body
        );
    }
    let put = fake
        .shell
        .calls()
        .into_iter()
        .find(|c| c.step == "subd.put")
        .unwrap();
    assert!(put.script.contains("mv -f"), "{}", put.script);

    // Файл пропал с хоста — publish докладывает его без новой версии.
    fake.state.borrow_mut().subs.remove(&token(&state, "bob"));
    fake.shell.clear();
    run_ok(&state, &fake.shell, &["publish"]);
    assert_eq!(fake.shell.count("subd.put"), 1);
    assert_eq!(state.load_user("bob").unwrap().version, 2);
    assert!(fake.state.borrow().subs.contains_key(&token(&state, "bob")));

    // Битые правила не публикуются.
    fs::write(state.rules_dir().join("broken.json"), "{\"id\": \"broken\"").unwrap();
    fake.shell.clear();
    let out = run_err(&state, &fake.shell, &["publish"]);
    assert!(out.contains("rules/broken.json"), "{out}");
    assert_eq!(fake.shell.count("subd.put"), 0);
    fs::remove_file(state.rules_dir().join("broken.json")).unwrap();

    let out = run_err(&state, &fake.shell, &["publish", "--host", "nosuch"]);
    assert!(out.contains("nosuch"), "{out}");
}

#[test]
fn verify_compares_local_build_with_host_files() {
    let (state, fake) = two_users("verify");
    let out = run_ok(&state, &fake.shell, &["verify"]);
    assert!(out.contains("совпадает: 2"), "{out}");
    assert_eq!(fake.shell.steps(), vec!["subd.hashes"]);

    // Подмена на хосте и лишний файл — verify падает и называет пользователя.
    let bob = token(&state, "bob");
    fake.state
        .borrow_mut()
        .subs
        .insert(bob.clone(), "{\"tampered\":1}".into());
    fake.state
        .borrow_mut()
        .subs
        .insert("x".repeat(32), "{}".into());
    let out = run_err(&state, &fake.shell, &["verify", "--host", "helsinki"]);
    assert!(out.contains("bob") && out.contains("расходится"), "{out}");
    assert!(out.contains("лишних файлов на хосте: 1"), "{out}");
    assert!(!out.contains(&bob), "токен не печатается:\n{out}");

    // После publish сходится снова; лишний файл остаётся лишним.
    run_ok(&state, &fake.shell, &["publish"]);
    let out = run_err(&state, &fake.shell, &["verify"]);
    assert!(out.contains("совпадает: 2"), "{out}");
    assert!(out.contains("лишних файлов на хосте: 1"), "{out}");
}
