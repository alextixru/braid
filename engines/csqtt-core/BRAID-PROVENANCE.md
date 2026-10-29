# Провенанс csqtt-core в Braid

## Источник

- Репозиторий: `~/work/dev/focsq` (remote `origin` = luminescq/focsq, `fork` = alextixru/focsq),
  каталог `backend/` — крейт `csqtt-core` 2.1.9, форк FOCSQ ядра amurcanov/csqtt.
- Ветка `feat/macos`, коммит `a485014a08ff67958b249d1332b8eec03dce0cb1`
  (2026-10-21, «fix(macos): заголовок окна — системный «светофор» вместо своих кнопок»).
- Скопирован каталог целиком, без `target/`: исходники, `shared/`, `vendor/primp`
  (обрезанная копия primp 1.3.1), `bin/` (`csqtt-cli`, `csqtt-tun-helper`), тесты,
  `proptest-regressions/`, `Cargo.lock`.
- Исходный репозиторий не менялся.

## Лицензия

PolyForm Noncommercial 1.0.0 (`SPDX-License-Identifier` в каждом файле, `license` в
`Cargo.toml`). Ядро протокола © 2026 amurcanov, порт FOCSQ © 2026 luminescq, macOS-порт
© 2026 alextixru. Лицензия действует на всё, что линкует `csqtt-core`: сборки Braid с CSQTT —
только некоммерческое использование. Дизайн, название и логотип FOCSQ в Braid не используются
(у них отдельные права, см. README апстрима). Код PWDTT не включён.

## Маркер правок

Каждое отличие от a485014 помечено `// [BRAID] …` (по образцу `[FOCSQ]`), SPDX-заголовки
апстрима сохранены. Новый файл `external.rs` — `SPDX-FileCopyrightText: 2026 alextixru`,
та же лицензия. Стек VK / TURN / obfs / worker / session не менялся.

## Как собирается в Braid

`engines/csqtt-core` — **не член** workspace `core/`, а path-зависимость крейта
`core/braid-engine-csqtt`. Причины: `make check-rust` гоняет `cargo fmt --check` и
`cargo clippy -D warnings` по членам workspace — вендоренный код им не подчиняется и не должен
переформатироваться; свой `Cargo.lock` даёт прогон тестов ядра `--locked`, как в CI апстрима.
Патч `primp = vendor/primp` действует только из корневого манифеста, поэтому он продублирован
в `core/Cargo.toml` (`[patch.crates-io]`).

- Тесты ядра: `cd engines/csqtt-core && cargo test --lib --locked` (апстрим: 329 passed,
  7 ignored; в Braid 340 passed — плюс 11 тестов `[BRAID]`). Из `core/` командой
  `cargo test -p csqtt-core` не запустить: cargo не тестирует не-члена workspace с
  dev-зависимостями — прогон ядра идёт отдельной строкой в `make check-rust`/CI.
- Проверка обёртки: `cd core && cargo check -p braid-engine-csqtt`,
  `cargo test -p braid-engine-csqtt`.
- C-зависимости (`aws-lc-sys`, `ring`, `zstd-sys`) собираются `cc` без cmake.

## Список правок

| Файл | Что | Зачем |
|---|---|---|
| `external.rs` (новый) | `EnginePort` (send → `Dispatcher::dispatch`, recv/колбэк ← очереди возврата, `stop`), `EngineHooksCore { on_config, on_exclude }`, `snapshot() -> CoreSnapshotPublic`, `CoreState`, `PortError`, глобальный слот хуков и статуса запуска, `CaptchaPendingGuard`, тесты | режим External: ядро внутри роутера без tun и UDP |
| `lib.rs` | `run_engine(ClientConfig, EnginePort, Arc<dyn EngineHooksCore>)`; общий `run_client_inner` (в External: `Dispatcher::start_external`, TUNCONF → `on_config`, без `start_parent_monitor` и `ctrl_c`, `native_tun = false`, события включены, старт групп пропускается при отмене до запуска); `pub const VK_EXCLUDE_CIDRS` (один список вместо трёх); `exclude_host_ip` — единая точка (хук или tun_*); поля `ClientConfig`: `turn_transport`, `core_dns`, `vk_profile`; `dns::set_dns_servers` до резолва пира; `mark_tunnel_inactive` при остановке; `schedule_vk_endpoint_excludes` без cfg по ОС; реэкспорт `SavedProfile` | критерии тикета 05 |
| `dispatcher.rs` | `ExternalUplink` (планировщик + нумератор потоков у хоста), `Dispatcher::start_external`, `write_external` (тот же порядок возврата, что у write_tun/write_udp) | вход/выход пакетов через порт |
| `dns.rs` | `set_dns_servers` / `parse_dns_list` / `dns_servers()` вместо константы Yandex; `mark_tunnel_inactive`; `tunnel_active` стал `pub(crate)` | DNS ядра из конфига, сброс `TUNNEL_ACTIVE` |
| `turn.rs` | `crate::exclude_host_ip` вместо трёх cfg-вызовов `tun_*::exclude_host_ip` | хук `on_exclude` |
| `tun_darwin.rs`, `tun_linux.rs`, `tun_win.rs` | локальные `VK_EXCLUDE_CIDRS` заменены на `use crate::VK_EXCLUDE_CIDRS` | один публичный список |
| `auth.rs` | `SavedProfileSource { Config, File }` в `VkAuth` вместо чтения `vk_profile.json` из cwd на каждой попытке | профиль из конфига; в External файл не читается |
| `events.rs` | `error(fatal=true)` отмечает код в статусе запуска; `network_suspect()` | `snapshot()`: состояние Dead и флаг сети |
| `captcha.rs` | `CaptchaPendingGuard` на время окна капчи | `snapshot()`: WaitingUser |

Не изменено: `session.rs`, `worker.rs`, `turn_core.rs`, `turn_stream.rs`, `turn_endpoint.rs`,
`obfs.rs`, `wrap.rs`, `vk_js_calls.rs`, `shared/*`, `vendor/primp`, `bin/*`, `Cargo.toml`,
`Cargo.lock`.
