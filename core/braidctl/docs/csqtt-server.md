# csqtt-server: откуда берётся, как ставится, как заводятся пользователи

Разведка апстрима `amurcanov/csqtt` для `braidctl host setup` и `add-user`
(ист. 48, 67). Всё ниже — из кода апстрима на закреплённом коммите; ничего из
клона не запускалось.

## Источник и пин

- Репозиторий: <https://github.com/amurcanov/csqtt> (архивный). Серверная часть —
  `rust-server/` (крейт `csqtt`, бинарь `csqtt`, `main.rs`, `model.rs`,
  `web_panel.rs`, `protocol.rs`), общий с клиентом код — `shared/`
  (`wire_protocol.rs`, `dtls.rs`, `spsc_ring.rs`). Лицензия — PolyForm
  Noncommercial 1.0.0 (как и у вендоренного `engines/csqtt-core`).
- **Пин: `b4ae5ce957b37d50414c91ae9afa96f2e98b68ac`** (2026-10-17, «Update
  README.md»; константа `setup::CSQTT_UPSTREAM_COMMIT`). Это последний коммит,
  у которого `shared/wire_protocol.rs` байт в байт совпадает с вендоренным
  `engines/csqtt-core/shared/wire_protocol.rs` (`WIRE_PROTOCOL_REVISION =
  "CSQTT-WIRE-3"`), а `rust-server/Cargo.toml` и `rust-client/Cargo.toml` —
  версии **2.1.9**. Такой же файл у «Initial commit» `71712b0` (2026-10-03);
  разница между ними — две строки README.
- Коммиты после пина («Update 2.5.0», 2026-10-20) меняют протокол: в
  `wire_protocol.rs` появляются фрагментация (`CSQTT_FRAG_MAGIC`, `Reassembler`)
  и `PATH_ECHO`, ревизия уезжает в `rust-server/main.rs`. С ядром 2.1.9 сервер
  2.5.0 несовместим — клиент получит `DENIED:protocol_mismatch` (`FATAL_PROTOCOL`).
- Проверка пина руками:
  `git hash-object engines/csqtt-core/shared/wire_protocol.rs` должен равняться
  `git -C <клон> rev-parse b4ae5ce:shared/wire_protocol.rs`. `setup-host.sh`
  дополнительно проверяет на хосте, что в файле есть `"CSQTT-WIRE-3"`, а в
  `rust-server/Cargo.toml` — `version = "2.1.9"`.

## Сборка

- `rust-server/rust-toolchain.toml`: `channel = "1.97.1"`, цель
  `x86_64-unknown-linux-musl`; `.cargo/config.toml` ставит `build.target` на musl
  с `link-self-contained=no`, а `build_linux.sh` собирает три musl-варианта через
  `cargo zigbuild` (нужны zig, pwsh) и кладёт их в `app/src/main/assets/`.
- Нам достаточно родного gnu-триплета хоста: `cd rust-server && cargo build
  --release --target "$(rustc -vV | sed -n 's/^host: //p')"` — `--target`
  перекрывает конфиг, zig и musl-tools не нужны. Зависимости сборки:
  `build-essential` (snmalloc-rs через `cc`, rusqlite bundled), `pkg-config`,
  `git`, `curl`; rustup сам подхватит 1.97.1 по `rust-toolchain.toml`.
- Бинарь понимает `--protocol-revision` (печатает `CSQTT-WIRE-3`) и `--version`
  (`csqtt 2.1.9`).

## Установка — установщик апстрима

Своего скрипта установки в репозитории сервера нет; его ставит Android-приложение
(`app/src/main/java/com/csqtt/client/ui/DeployOperations.kt`) скриптом
`app/src/main/assets/deploy.sh` (2278 строк, `SCRIPT_VERSION=2.1.9`,
`CSQTT_WIRE_PROTOCOL_REVISION=CSQTT-WIRE-3`). `host setup` повторяет ровно этот
контракт (`deploy/setup-host.sh`, шаг 5):

1. На хост кладутся одноразовые файлы:
   `/tmp/.csqtt-upload-server` — бинарь;
   `/tmp/.csqtt-upload-web.env` — `CSQTT_WEB_USER=…`, `CSQTT_WEB_PASS=…`;
   `/tmp/.csqtt-upload-overrides.json` — `{"main_password": "…", "device_id": "…"}`.
2. Запуск от root: `env CSQTT_PEER_PORT=46000 CSQTT_SSH_PORT=22
   CSQTT_WEB_PORT=46002 CSQTT_DEPLOY_MODE=systemd bash deploy.sh install`
   (`uninstall`, `status` — тоже есть; `CSQTT_DEPLOY_MODE=docker` — контейнер).
3. Результат — маркеры в stdout: `CSQTT_PROGRESS|<доля>|<текст>`,
   `CSQTT_DEPLOY_OK`, `CSQTT_DEPLOY_ERROR|<фаза>|<сообщение>`,
   `CSQTT_DEPLOY_ROLLED_BACK`; лог — `/var/log/csqtt-install.log`.

Что делает `deploy.sh install` (Debian/Ubuntu, systemd): ставит пакеты;
`sysctl net.ipv4.ip_forward=1`; проверяет бинарь (`--protocol-revision` ==
`CSQTT-WIRE-3`, иначе отказ до остановки старого сервера); iptables с меткой-комментарием
(INPUT udp peer-порт, tcp web- и ssh-порты, FORWARD, `MASQUERADE` для
`10.66.67.0/24` через WAN-интерфейс, MSS clamp) и помощники
`/usr/local/lib/csqtt/network-up.sh`, `tun-recover.sh`; бинарь →
`/usr/local/bin/csqtt`; `/etc/csqtt` (0700) с `csqtt.env` (0600) и
`deploy-overrides.json` (0600, сервер читает его при первом старте и удаляет);
самоподписанный сертификат панели (`web_cert.pem`/`web_key.pem`, rcgen; при
возможности — Let's Encrypt для IP с таймером обновления); юнит
`/etc/systemd/system/csqtt.service`:

```
EnvironmentFile=-/etc/csqtt/csqtt.env
Environment=CSQTT_SERVICE_MANAGER=systemd
ExecStartPre=/usr/local/lib/csqtt/network-up.sh
ExecStartPre=/usr/local/lib/csqtt/tun-recover.sh
ExecStart=/usr/local/bin/csqtt --listen 0.0.0.0:46000 --web-port 46002 --config-dir /etc/csqtt
Restart=on-failure
```

Интерфейс сервера — `csqtt1`, адрес `10.66.67.1`, клиентам — `10.66.67.x`
(`TUNCONF:<ip>:<dns>`). База и env переживают переустановку и даже `uninstall`.
AWG (контейнер Amnezia или нативный `awg`) установщик не трогает: свои правила он
помечает комментарием и удаляет только их.

## Конфиг и состояние (`--config-dir /etc/csqtt`)

- `csqtt.db` — SQLite (`model.rs: Database`): `main_password`, `main_device_id`,
  `dns`, `passwords{пароль → PasswordEntry{device_id, expires_at, name,
  vk_hashes, ports, is_deactivated, dtls_port, wg_port, local_port, …}}`,
  `devices{device_id → ClientDevice{ip, pub_key, …}}`, настройки панели,
  профили локального SOCKS5. Старый `passwords.json` — одноразовая миграция.
- `csqtt.env` — `CSQTT_WEB_USER`, `CSQTT_WEB_PASS` (логин панели; без них сервер
  печатает случайный пароль в журнал при каждом старте).
- `deploy-overrides.json` — `main_password`, `device_id` (одноразовый).
- `web_cert.pem`, `web_key.pem` — TLS панели.
- Аргументы сервера: `--listen 0.0.0.0:46000`, `--web-port 46002`,
  `--config-dir`, `--password`/`CSQTT_MAIN_PASSWORD`, `--device-id`/`CSQTT_DEVICE_ID`,
  `--web-user`, `--web-pass`, `--dns IP[,IP]` (по умолчанию 77.88.8.8,77.88.8.1),
  `--secure-cookie`, `--fec safe|off`; подкоманды `start|stop|restart|dpi|help`.

## Пользователи = пароли, заводятся через веб-панель

CLI для клиентов у сервера **нет** — только веб-панель (`axum`, `web_panel.rs`),
слушает `0.0.0.0:<web-port>` по HTTPS:

- `POST /api/login` `{"user","pass"}` (значения можно слать как есть или с
  префиксом `c1:` + base64 сдвига; сервер сравнивает в constant-time) →
  cookie `csqtt_session=…; Max-Age=86400; HttpOnly; Secure`. Три неверных
  попытки с адреса — временная блокировка (`429`).
- `GET /api/clients` → `[{password, name, device_id, ip, active,
  active_sessions, expires, down, up, vk_hash, vk_hashes, ports, dtls_port,
  wg_port, local_port}]` (основной пароль — в том же списке).
- `POST /api/clients` `{"name": "...", "days": 0..3650 (0 — бессрочно),
  "hash": "" | "h1,h2" (1–6 хешей VK без пробелов), "dtls_port"?, "wg_port"?,
  "local_port"?}` → `{"password", "expires", "dtls_port", "wg_port",
  "local_port", "vk_hashes", "reload_error"}`. Пароль — 16 символов из
  `ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789`; создаётся с пустым
  `device_id`. Лимит — `MAX_PASSWORDS = 20` на сервер (просроченные вычищаются).
- `POST /api/clients/{password}` — изменить (`name`, `days`, хеши, порты);
  `DELETE /api/clients/{password}` — удалить (сессии сбрасываются);
  `POST /api/clients/{password}/toggle` — деактивировать/включить;
  `POST /api/clients/{password}/unbind` — отвязать от устройства.
- `POST /api/settings` `{main_password?, dns_provider? ("yandex"|"google"),
  dns_primary?, dns_secondary?, auto_restart_interval_hours?}`; `GET /api/stats`,
  `/api/logs`, `/api/reboot`.

### Привязка к устройству (`protocol.rs: getconf_credential_access`)

Клиент первым пакетом сессии шлёт `GETCONF:<port>|<device_id>|<password>|<gen>|<salt>|<worker>|<count>|CSQTT-WIRE-3`.
Сервер:

- пароль равен `main_password`: если `main_device_id` задан и не совпал —
  `DENIED:device_mismatch`, иначе доступ без привязки;
- пароля нет в базе или истёк — `DENIED:expired`; деактивирован —
  `DENIED:deactivated`;
- у пароля задан `device_id`, и он не совпал — `DENIED:device_mismatch`;
- `device_id` пустой — пароль привязывается к устройству при первом успешном
  подключении (`Unbound → Bound`).

То есть **один пароль — одно устройство**. Второму устройству того же человека
нужен свой профиль (`add-user <имя>-phone`) или `unbind` в панели.

## Что делает `braidctl`

- `host setup <id>` → `deploy/setup-host.sh` на хосте: клон апстрима на пине,
  сборка `rust-server`, одноразовые файлы установщика из
  `state/secrets/<host>/_host.toml` (логин и пароль панели, основной пароль —
  генерируются один раз, в `deploy-overrides.json` `device_id` пустой: основной
  пароль ни к чему не привязан, braidctl им не пользуется), `deploy.sh install`,
  затем `braid-subd` своим `install.sh`. Повтор без изменений: бинарь совпадает —
  установщик не запускается.
- `host import <id>` → `csqtt.detect`: `/usr/local/bin/csqtt --version`,
  `systemctl cat csqtt` (`--listen`, `--web-port`, `--config-dir`), `is-active`.
- `add-user <имя> --host <id>` → `csqtt.clients` (есть ли сохранённый пароль на
  сервере) и при необходимости `csqtt.create` (`name=<имя>`, `days=0`,
  `hash=""`); пароль — в `state/secrets/<host>/<имя>.toml`, в подписке —
  `engines[]: {type: csqtt, peer: <public_ip>:46000, password, workers, obfs,
  fingerprint}`. Хеши VK пользователь вводит в приложении.
- `revoke <имя>` → `DELETE /api/clients/<пароль>`.
- Все вызовы панели выполняются на самом хосте (`curl -k
  https://127.0.0.1:<web_port>/api/…`), логин и пароль читаются из
  `/etc/csqtt/csqtt.env` — наружу не передаются и не печатаются.
