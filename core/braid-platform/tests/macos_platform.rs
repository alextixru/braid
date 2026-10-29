//! `MacosPlatform` без помощника и без root: правило sudoers, выбор способа
//! запуска, LaunchAgent автозапуска, идентификатор сети, исходные DNS.
//! Всё с помощником — в тестах `braid-helper` (там есть бинарь для `--dry-run`).
#![cfg(target_os = "macos")]

use braid_platform::Platform;
use braid_platform::macos::MacosPlatform;
use braid_platform::macos::autostart::launch_agent_plist;
use braid_platform::macos::launch::{
    LaunchMethod, LaunchPlan, SystemLauncher, choose_launch_method, helper_path_from,
    osascript_script, sudo_command,
};
use braid_platform::macos::network::{NetworkProbe, dns_servers_from, network_identity};
use braid_platform::macos::sudoers::{sudoers_install, sudoers_install_at};
use braid_proto::NetworkKind;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "braid-platform-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Ревью тикета 08: NOPASSWD-правило не может указывать на бинарь в `.app`
/// пользователя (подмена файла между проверкой дайджеста и execve) —
/// правило ставится на копию в каталоге root, туда же бинарь копирует та
/// же команда установки, и оттуда идёт запуск по sudo.
#[test]
fn sudoers_rule_points_to_root_owned_copy_not_the_bundle() {
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let bundled = Path::new("/Applications/Braid.app/Contents/MacOS/braid-helper");
    let plan = sudoers_install("alex", bundled, digest);
    assert_eq!(plan.path, Path::new("/etc/sudoers.d/braid"));
    assert_eq!(
        plan.installed_helper,
        Path::new("/Library/PrivilegedHelperTools/braid-helper")
    );
    assert!(
        plan.rule.lines().any(|line| line
            == format!(
                "alex ALL=(root) NOPASSWD: sha256:{digest} /Library/PrivilegedHelperTools/braid-helper"
            )),
        "правило: {}",
        plan.rule
    );
    assert!(
        !plan.rule.contains("/Applications"),
        "правило не ссылается на каталог пользователя: {}",
        plan.rule
    );
    // Установка одной командой от root: копия → сверка дайджеста → root:wheel 0755
    // → на место → правило через visudo -cf → 0440. Порядок важен.
    let cmd = &plan.shell_command;
    let copy_at = cmd
        .find("/bin/cp -f '/Applications/Braid.app/Contents/MacOS/braid-helper' '/Library/PrivilegedHelperTools/braid-helper.tmp'")
        .expect("копирование бинаря в каталог root");
    let digest_at = cmd
        .find("/usr/bin/shasum -a 256")
        .expect("сверка дайджеста копии");
    assert!(cmd[digest_at..].contains(digest));
    let chown_at = cmd
        .find("/usr/sbin/chown root:wheel '/Library/PrivilegedHelperTools/braid-helper.tmp'")
        .expect("копия принадлежит root");
    let chmod_at = cmd
        .find("/bin/chmod 0755 '/Library/PrivilegedHelperTools/braid-helper.tmp'")
        .expect("копия без записи для пользователя");
    let visudo_at = cmd.find("/usr/sbin/visudo -cf").expect("проверка правила");
    assert!(copy_at < digest_at && digest_at < chown_at && chown_at < chmod_at);
    assert!(
        chmod_at < visudo_at,
        "правило пишется после установки копии"
    );
    assert!(cmd.contains("/bin/chmod 0440"));
    assert!(cmd.ends_with("/etc/sudoers.d/braid"));
    assert!(plan.osascript.starts_with("do shell script \""));
    assert!(plan.osascript.ends_with("\" with administrator privileges"));

    // Пробел в пути копии экранируется по правилам sudoers.
    let custom = sudoers_install_at(
        "alex",
        bundled,
        Path::new("/Library/Privileged Tools/braid-helper"),
        Path::new("/etc/sudoers.d/braid"),
        digest,
    );
    assert!(
        custom.rule.contains(&format!(
            "NOPASSWD: sha256:{digest} /Library/Privileged\\ Tools/braid-helper"
        )),
        "{}",
        custom.rule
    );
}

/// Запуск по sudo — только копия из каталога root и только пока она
/// совпадает с помощником в `.app`; иначе диалог пароля с бинарём из `.app`.
#[test]
fn launch_plan_uses_root_copy_for_sudo_and_bundle_for_the_dialog() {
    let dir = temp_dir("plan");
    let bundled = dir.join("Braid.app/Contents/MacOS/braid-helper");
    std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
    std::fs::write(&bundled, b"helper v1").unwrap();
    let installed = dir.join("PrivilegedHelperTools/braid-helper");
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::write(&installed, b"helper v1").unwrap();
    let rule = dir.join("sudoers.d/braid");
    std::fs::create_dir_all(rule.parent().unwrap()).unwrap();
    std::fs::write(&rule, "rule").unwrap();
    let socket = Path::new("/tmp/h.sock");
    let launcher = SystemLauncher::new(&bundled)
        .with_installed_path(&installed)
        .with_sudoers_path(&rule);

    match launcher.plan(socket, 501) {
        LaunchPlan::Sudo(command) => {
            assert_eq!(command[0], "/usr/bin/sudo");
            assert_eq!(command[3], installed.to_string_lossy());
            assert!(
                !command.iter().any(|arg| arg.contains("Braid.app")),
                "sudo не запускает бинарь из .app: {command:?}"
            );
        }
        other => panic!("ожидался запуск по sudo, получили {other:?}"),
    }

    // Приложение обновилось, копия отстала — без sudo, диалог с бинарём из .app.
    std::fs::write(&installed, b"helper v0").unwrap();
    match launcher.plan(socket, 501) {
        LaunchPlan::Osascript(script) => {
            assert!(
                script.contains(&format!("'{}'", bundled.display())),
                "{script}"
            );
            assert!(!script.contains("PrivilegedHelperTools"), "{script}");
        }
        other => panic!("ожидался диалог пароля, получили {other:?}"),
    }

    // Правила нет — тоже диалог.
    std::fs::remove_file(&rule).unwrap();
    assert!(matches!(
        launcher.plan(socket, 501),
        LaunchPlan::Osascript(_)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `BRAID_HELPER` — только для отладочных сборок; в релизе путь помощника
/// всегда рядом с исполняемым файлом приложения.
#[test]
fn helper_env_override_only_in_debug_builds() {
    let exe = PathBuf::from("/Applications/Braid.app/Contents/MacOS/Braid");
    assert_eq!(
        helper_path_from(Some(PathBuf::from("/tmp/evil")), Some(exe.clone()), false),
        Path::new("/Applications/Braid.app/Contents/MacOS/braid-helper")
    );
    assert_eq!(
        helper_path_from(
            Some(PathBuf::from("/tmp/dev-helper")),
            Some(exe.clone()),
            true
        ),
        Path::new("/tmp/dev-helper")
    );
    assert_eq!(
        helper_path_from(None, Some(exe), true),
        Path::new("/Applications/Braid.app/Contents/MacOS/braid-helper")
    );
}

#[test]
fn launch_method_prefers_sudo_when_rule_is_installed() {
    assert_eq!(choose_launch_method(0, false), LaunchMethod::Direct);
    assert_eq!(choose_launch_method(501, true), LaunchMethod::Sudo);
    assert_eq!(choose_launch_method(501, false), LaunchMethod::Osascript);

    let helper = Path::new("/Applications/Braid's \"dev\".app/braid-helper");
    let socket = Path::new("/Users/alex/Library/Application Support/Braid/h.sock");
    assert_eq!(
        sudo_command(helper, socket, 501),
        [
            "/usr/bin/sudo",
            "-n",
            "--",
            "/Applications/Braid's \"dev\".app/braid-helper",
            "serve",
            "--sock",
            "/Users/alex/Library/Application Support/Braid/h.sock",
            "--uid",
            "501",
        ]
    );
    assert_eq!(
        osascript_script(helper, socket, 501),
        r#"do shell script "'/Applications/Braid'\\''s \"dev\".app/braid-helper' serve --sock '/Users/alex/Library/Application Support/Braid/h.sock' --uid 501 >/dev/null 2>&1 &" with administrator privileges"#
    );
}

#[test]
fn launch_agent_plist_is_valid_and_set_autostart_writes_it() {
    let plist = launch_agent_plist(
        "org.braid.app",
        Path::new("/Applications/Braid.app/Contents/MacOS/Braid"),
    );
    assert!(plist.contains("<key>Label</key>\n\t<string>org.braid.app</string>"));
    assert!(plist.contains("<string>/Applications/Braid.app/Contents/MacOS/Braid</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));

    let dir = temp_dir("agents");
    let platform = MacosPlatform::new()
        .with_launch_agent_dir(&dir)
        .with_program("/Applications/Braid.app/Contents/MacOS/Braid")
        .with_launchctl(false);
    assert!(!platform.autostart_enabled());
    platform.set_autostart(true).unwrap();
    let path = dir.join("org.braid.app.plist");
    assert!(path.is_file());
    assert!(platform.autostart_enabled());
    let lint = Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        lint.status.success(),
        "{}",
        String::from_utf8_lossy(&lint.stdout)
    );
    platform.set_autostart(false).unwrap();
    assert!(!path.exists());
    assert!(!platform.autostart_enabled());
    // Повторное выключение безвредно.
    platform.set_autostart(false).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn network_identity_hashes_ssid_or_gateway_and_never_leaks_them() {
    let wifi = NetworkProbe {
        primary_interface: Some("en0".into()),
        router: Some(Ipv4Addr::new(10, 241, 8, 194)),
        wifi: true,
        ssid: Some("HomeNet".into()),
        gateway_mac: Some("AE:41:43:B2:FC:4C".into()),
    };
    // FNV-1a 64 от "HomeNet", посчитан отдельно (python), не кодом платформы.
    let (id, kind) = network_identity(&wifi).unwrap();
    assert_eq!(id.as_str(), "wifi:3d7880f113ad7b51");
    assert_eq!(kind, NetworkKind::Wifi);

    // Без SSID (нет разрешения) — шлюз + MAC, MAC нормализуется к нижнему регистру.
    let no_ssid = NetworkProbe {
        ssid: None,
        ..wifi.clone()
    };
    let (id, kind) = network_identity(&no_ssid).unwrap();
    assert_eq!(id.as_str(), "gw:6e1cd282dbe93289");
    assert_eq!(kind, NetworkKind::Wifi);
    assert!(
        !id.as_str().contains("10.241"),
        "в идентификатор не попадает адрес"
    );

    let wired = NetworkProbe {
        wifi: false,
        ssid: None,
        ..wifi.clone()
    };
    assert_eq!(network_identity(&wired).unwrap().1, NetworkKind::Wired);

    let offline = NetworkProbe {
        primary_interface: None,
        ..wifi
    };
    assert!(network_identity(&offline).is_none());
}

#[test]
fn original_dns_comes_from_server_addresses_without_our_proxy() {
    let value = serde_json::json!({
        "ServerAddresses": ["172.29.172.254", "10.255.0.1", "garbage", "2001:db8::1"],
        "SearchDomains": ["lan"]
    });
    let peer: IpAddr = Ipv4Addr::new(10, 255, 0, 1).into();
    assert_eq!(
        dns_servers_from(&value, &[peer]),
        vec![
            IpAddr::from(Ipv4Addr::new(172, 29, 172, 254)),
            "2001:db8::1".parse::<IpAddr>().unwrap()
        ]
    );
    assert!(dns_servers_from(&serde_json::json!({}), &[]).is_empty());
}

#[test]
fn capabilities_are_desktop_without_per_app_rules() {
    let platform = MacosPlatform::new();
    let caps = platform.capabilities();
    assert!(!caps.per_app);
    assert!(caps.menu_bar);
    assert!(!caps.qr_scan);
    assert!(platform.protect_socket(3), "без VpnService защищать нечего");
}
