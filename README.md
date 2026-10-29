# Braid

Общий VPN-клиент для macOS и Android: несколько движков (AmneziaWG, CSQTT и слот
под третий) к одному хосту, один tun-интерфейс, правила маршрутизации подпиской.
Роутер сам выбирает живой движок, домены из правил идут в туннель, остальное — мимо.

## Карта репозитория

```
app/                      Flutter-приложение (macOS, Android; Linux и Windows — заготовки)
  lib/                    экраны и сервисы; lib/src/rust — сгенерированный мост (в git)
  rust/                   braid_ffi — крейт моста flutter_rust_bridge
  rust_builder/           FFI-плагин с cargokit: Rust собирается внутри flutter build
core/                     Cargo workspace
  braid-proto/            общие типы: схема подписки, ссылка braid://, события журнала
  braid-core/             роутер и всё платформонезависимое ядро (Core — один на процесс)
  braid-platform/         трейт Platform: macos/, android/, заглушки linux/ и windows/, memory для тестов
  braid-engine-awg/       движок AWG — обёртка над libawg (появится)
  braid-engine-csqtt/     движок CSQTT — обёртка над engines/csqtt-core (появится)
  braid-helper/           root-помощник macOS (появится)
  braid-subd/             сервер подписок и телеметрии (появится)
  braidctl/               CLI админа (появится)
engines/
  awg-go/                 libawg: cgo c-shared над amneziawg-go (появится)
  csqtt-core/             вендоренная копия csqtt-core с правками [BRAID] (появится)
tools/                    скрипты: генераторы списков, конвертер Amnezia → правила, split для zen
docs/                     architecture.md, adr/
state/                    данные админа — ВНЕ git: rules/*.json, modes.json, hosts/, users/, secrets/
archive/                  дампы и исходные списки Amnezia — вне git
.github/workflows/        check.yml — тесты; build.yml — .app и APK (появится)
```

В git — только код и документация: ни адресов хостов, ни имён пользователей,
ни списков доменов. `state/`, `archive/`, `.autopilot/` игнорируются.

## Команды

Нужны Rust (stable), Go и Flutter 3.47 (`~/development/flutter`, Makefile добавляет его в PATH).

| Команда | Что делает |
|---|---|
| `make setup` | rustfmt и clippy, цели Rust, `flutter_rust_bridge_codegen`, `flutter pub get`, `go mod download` |
| `make check` | всё, что гоняет CI: fmt, clippy и тесты Rust, кросс-проверка заглушек платформ, go vet/test, flutter analyze и тесты |
| `make check-rust` / `check-flutter` / `check-cross` / `check-go` | те же проверки по частям |
| `make codegen` | перегенерировать мост после правки `app/rust/src/api` |
| `make fmt` | отформатировать Rust и Dart |
| `make libawg`, `make helper`, `make subd`, `make ctl` | компонентные сборки |
| `make macos`, `make apk`, `make release` | сборки приложения — нужны Xcode и Android SDK, их делает CI |

Один тест: `cd core && cargo test -p braid-proto link` · `cd app && flutter test test/widget_test.dart`.

## Правила и режимы (`state/`)

`state/rules/<id>.json` — набор правил в формате `RuleSet` (`braid-proto`):

```json
{ "id": "cursor", "name": "Cursor",
  "domains": ["cursor.com", "*.cursor.com"], "ips": ["104.18.0.0/16"],
  "exclude_domains": [], "exclude_ips": [] }
```

`*.x.y` — любой поддомен на любой глубине (многоуровневые `*.*.x.y` из списков Amnezia
принимаются как есть); исключения побеждают включения; только IPv4.
`state/modes.json` — пресеты: `{ "modes": [ { "id", "name", "sets": [...], "exclude_domains", "exclude_ips", "default" } ] }`,
ровно один режим с `"default": true`.

Перенос списка Amnezia: `python3 tools/amnezia_to_rules.py archive/amnezia_x.json x "Имя"`.
Split-туннель для сервера zen: `python3 tools/gen_zen_vpnsplit.py <папка> <endpoint-IP>` — читает `state/rules/*.json`;
свои подсети «никогда в туннель» — `state/never.txt` (по CIDR на строку, `#` — комментарий).

## Лицензии

Компоненты и их лицензии — `THIRD_PARTY.md`. `csqtt-core` распространяется по
PolyForm Noncommercial 1.0.0: сборки Braid, которые его включают, — только для себя и своих.
