<!-- autopilot:start -->
# Braid — общий VPN-клиент для macOS и Android (Flutter + Rust), для себя и своих
Один tun, роутер с fake-IP, движки AmneziaWG и CSQTT, автовыбор, подписка по HTTPS. Устройство — `docs/architecture.md`, почему так — `docs/adr/`, словарь — `CONTEXT.md`. Комментарии, логи, коммиты, тексты UI — по-русски, идентификаторы — по-английски.

## Команды
| Команда | Что делает |
|---------|------------|
| `export PATH="$HOME/development/flutter/bin:$PATH"` | перед любой командой Flutter/Dart — в PATH его нет (`make` добавляет сам) |
| `make setup` | rustfmt и clippy, цели Rust, `flutter_rust_bridge_codegen` 2.12.0, `flutter pub get`, `go mod download`; нужны Rust 1.99, Go 1.27, Flutter 3.47.7 |
| `make check` | вся проверка как в CI (`.github/workflows/check.yml`), ~5 мин; части: `check-rust` `check-cross` `check-go` `check-flutter` `check-csqtt` `check-version`; все цели — `make help` |
| `cd core && cargo test -p braid-core --test router_nat udp_to_fake_ip` | один тест Rust: `--test <файл из tests/>` и имя; сквозной — `cargo test -p braid-runtime --test e2e` |
| `cd app && flutter test test/rules_test.dart` | один тест Flutter; Go — `cd engines/awg-go && go test ./... -run TestPktTun` |
| `make ctl subd helper` | `braidctl`, `braid-subd`, `braid-helper` в `core/target/release/`; `make libawg` — universal-библиотека AWG в `engines/awg-go/dist/darwin/` |
| `make codegen` | мост flutter_rust_bridge после правки `app/rust/src/api/`; результат (`app/lib/src/rust/`, `app/rust/src/frb_generated.rs`) лежит в git |
| `make macos` · `make apk` · `make libawg-android` | только в CI (`.github/workflows/build.yml`): нужны Xcode, JDK, NDK — локально их нет |

## Где что
- Ядро — `core/braid-core/src/core/mod.rs`: фасад `Core` в конце файла, логика сессии и `tick` — `impl Shared` выше; соседние модули (`router` → `dns` → `nat` — путь пакета, `health` + `policy` — автовыбор) — карта в `docs/architecture.md`.
- Тесты ядра — `core/braid-core/tests/` на подделках из `core/braid-core/src/testing/`; `Core` глобален, поэтому `Core::init`/`build_core` — раз на тестовый бинарь, в остальных тестах `Core::with_deps`.
- Движки — трейт `core/braid-core/src/engine.rs`; AWG: `core/braid-engine-awg` над `engines/awg-go` (Go, C-API `engines/awg-go/awg.h`); CSQTT: `core/braid-engine-csqtt` над вендоренным `engines/csqtt-core` (правки только с `// [BRAID]`, источник — `engines/csqtt-core/BRAID-PROVENANCE.md`); фабрика и `build_core` — `core/braid-runtime/src/lib.rs`.
- Платформа — трейт `core/braid-platform/src/platform.rs`; `macos` — клиент root-помощника `core/braid-helper` (протокол `core/braid-platform/src/macos/protocol.rs`), `android` — JNI, а входы, которым нужно ядро, — `core/braid-runtime/src/android.rs`; нативный слой — Kotlin `app/android/app/src/main/kotlin/org/braid/vpn/` и Swift `app/macos/Runner/`.
- Мост — `app/rust/src/api/` (то, что видит Dart; потом `make codegen`), логика рядом в `app/rust/src/`; Dart-сторона — `app/lib/bridge/`, `app/lib/controller/braid_controller.dart`, `app/lib/models/`; эталоны JSON `app/test/fixtures/` сверяют `app/rust/src/contract.rs` и Dart-тесты — правь обе стороны.
- Экраны — `app/lib/screens/` (по папке на экран; вход `app/lib/main.dart` → `app/lib/app.dart` → `app/lib/screens/gate.dart`); «Движки», «Статистика», «О программе», капча и запуск трея (`app/lib/tray/`) подключены слотами `app/lib/app_extensions.dart`; подделки для тестов — `app/test/support/` (`FakeBridge`, `pumpApp`).
- Админ и хост — CLI `core/braidctl/src/cli.rs` (хосты по SSH через трейт `Shell` в `core/braidctl/src/shell.rs`, `--dry-run` только печатает; перед первой командой нужен `braidctl init`); сервер подписок `core/braid-subd/src/` и `core/braid-subd/deploy/`; схема подписки и `braid://` — `core/braid-proto/src/`; установка на хост — `core/braidctl/deploy/setup-host.sh`.

## Подводные камни
- Все цели — `make help`; `make` сам добавляет Flutter и cargo в PATH (голый `flutter` — нет); `check-flutter` собирает мост перед `flutter test`, а без моста (`cd app/rust && cargo build` или `BRAID_FFI_LIB`) `app/test/ffi_bridge_test.dart` пропускается.
- Rust форматируй только `make fmt`: `cargo fmt --all` в `core/` переписывает вендоренный `engines/csqtt-core`.
- Три корня Cargo со своими lock-файлами: `core/` (workspace), `app/rust/` (мост: `cd app/rust && cargo test`), `engines/csqtt-core/` (`cargo test --lib --locked`, из `core/` не запускается); у `braid-core` нет тестов в `src`, поэтому фильтр вида `nat::` находит 0 тестов — бери `--test <файл>`.
- Kotlin, Swift, `.app` и APK локально не собираются (нет Xcode, JDK, Android SDK и NDK): Kotlin-тесты и `cargo check -p braid-runtime --target aarch64-linux-android` идут только в CI; `gradlew` в git нет — в свежем клоне сперва `cd app && flutter build apk --config-only`.
- Версия — файл `VERSION`; менять только `make set-version V=x.y.z` (`core/Cargo.toml`, `app/rust/Cargo.toml`, их lock-файлы и `app/pubspec.yaml`), сверяет `make check-version`.
- Тесты-сторожа читают исходники: копия `THIRD_PARTY.md` лежит в `app/assets/third_party.md` (после правки оригинала — `cp THIRD_PARTY.md app/assets/third_party.md`); имя приложения в `app/lib` — только константа `appName` из `app/lib/app.dart`, литерал «Braid» роняет `app/test/app_name_test.dart`.
- Нестабильны — при падении перезапусти: `telemetry_body_over_one_megabyte_is_413` в `core/braid-subd/tests/http.rs` (гонка 413) и три теста `udp_batch::tests` в `engines/csqtt-core` (`sends_and_receives_*`, `try_send_reports_*`).
- Репозиторий публичный: `.autopilot/`, `state/` (данные и секреты админа), `archive/` — вне git; в коммитах ни адресов хостов, ни имён пользователей, списков доменов и ключей. В скриптах для хоста секреты идут только через `stdin` (`Cmd` в `core/braidctl/src/shell.rs`), значения из `hosts/*.toml` — через `sh_quote`.
- Тесты — без сети и привилегий; `braidctl` ходит на реальные хосты по SSH (`host setup`, `add-user`, `publish`, …) — только с подтверждения пользователя и сначала с `--dry-run`.

## Окружение (для `make setup` и `make check` переменные не нужны)
- `ANDROID_NDK_HOME` — NDK для `make libawg-android`; `BRAID_AWG_DIST` — каталог с готовой `libawg` вместо `engines/awg-go/dist/<ОС>`: тогда `make -C engines/awg-go` не вызывается.
- `BRAID_FFI_LIB` — путь к собранной библиотеке моста `libbraid_ffi` для `app/test/ffi_bridge_test.dart` (иначе он ищет в `app/rust/target/debug` и `core/target/debug`).
- `BRAID_STATE` — каталог `state/` для `braidctl` (как `--state`; иначе `state/` в корне репозитория); `RUST_LOG` — уровень логов `braid-subd`.
- `BRAID_PUBLIC_IP` (обязательна) и прочие `BRAID_*` из шапки `core/braidctl/deploy/setup-host.sh` — скрипт читает их на хосте, `braidctl host setup` передаёт сам; `CSQTT_*` — переключатели вендоренного ядра (отладка, soak, живые e2e под `#[ignore]` — не запускать).

## Как здесь работает Autopilot

Сборка ведётся навыком `/autopilot`. Требования, спецификация и таски — в `.autopilot/`.
Прогресс — `.autopilot/dashboard.html`. Требование из `manifest.md` может снять
только пользователь. Прерванную сборку продолжает «продолжи автопилот».

### Правка без автопилота

Мелкий баг или доводка после сборки — обычная сессия, без прогона.

Сначала прочитай:
1. `.autopilot/PROGRESS.md` — что не доделано, известные баги, о чём договорились с пользователем.
2. Что правили после последней сборки — первая строка таблицы в «История git».
3. Только если правка их касается: почему сделано так — `docs/adr/`; что значат слова — `CONTEXT.md`;
   что именно просили — `manifest.md` в папке сборки (указана в начале `PROGRESS.md`).

После правки:
- коммит — по правилам из «История git»;
- строка в «Правки без автопилота» в `PROGRESS.md`: дата — что — почему;
- починенный пункт из «Не доделано» или «Известные баги» — удали;
- договорились с пользователем о том, что следующая сборка не должна пересматривать, —
  строка в «Договорились», его словами.

### История git

Целиком не читай — бери срез под вопрос:

| Вопрос | Команда |
|---|---|
| что правили после последней сборки | `git log --oneline "$(git log -1 --format=%h --grep='^Autopilot: сборка сдана')"..HEAD -- . ':(exclude).autopilot'` |
| всё, что делали ради требования | `git log --oneline --grep="R02"` |
| история файла | `git log --oneline -10 -- <файл>` |
| когда появился или исчез код | `git log --oneline -S "<фрагмент>"` |
| почему строка такая | `git blame -L <с>,<по> <файл>`, затем `git show <sha>` |
| что внутри коммита | `git show --stat <sha>` |

Срез длиннее ~30 коммитов — отдай его субагенту и возьми выжимку в десять строк.

Коммит: заголовок — что (`fix: даты в одном формате`), тело — почему, трейлер
`--trailer "Requirements: R02"`, если правка касается требования из `manifest.md`.
Заголовок `Autopilot: сборка сдана` пишет только автопилот — по нему находят последнюю сборку.
<!-- autopilot:end -->
