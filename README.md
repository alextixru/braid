# Braid

Общий VPN-клиент для macOS и Android: несколько движков (AmneziaWG, CSQTT и слот под
третий) к одному хосту, один tun-интерфейс, правила маршрутизации подпиской. Роутер сам
выбирает живой движок (приоритет, тёплый запас, память сети), домены из правил идут в
туннель, остальное — мимо. Админ управляет правилами, хостами и пользователями из CLI
`braidctl`; только его устройства ведут журнал и шлют телеметрию.

Как это устроено — [`docs/architecture.md`](docs/architecture.md), почему именно так —
[`docs/adr/`](docs/adr/). Компоненты и лицензии — [`THIRD_PARTY.md`](THIRD_PARTY.md).

## Установка

Сборки лежат в GitHub Releases (`Braid-<версия>-macos-universal.zip`,
`Braid-<версия>-android.apk`); их делает GitHub Actions по тегу `v*`. Без личной ссылки
`braid://…` от админа подключаться нечем — первый экран просит вставить ссылку или
отсканировать QR.

### macOS 12+ (Intel и Apple Silicon)

1. Распакуй zip и перенеси `Braid.app` в «Программы».
2. Первый запуск — правой кнопкой по приложению → «Открыть» → «Открыть» в диалоге:
   подпись ad-hoc, без нотаризации, иначе Gatekeeper молча откажет.
3. Вставь ссылку (или открой `braid://…` из письма — приложение зарегистрировано на схему)
   и нажми «Подключить». macOS один раз спросит **пароль администратора**: интерфейс utun
   создаёт root-помощник внутри приложения.
4. Чтобы пароль не спрашивался при каждом подключении, в «Настройках» установи правило
   sudoers — только с твоего согласия, это отдельная кнопка. Правило привязано к хешу
   помощника: после обновления приложения баннер предложит переустановить его.

Значок в меню-баре показывает движок и состояние; закрытие окна не завершает приложение.
Другой VPN, который держит DNS и маршруты (например, Amnezia), надо отключить —
Braid назовёт мешающий интерфейс и не стартует молча.

### Android 10+

1. Поставь APK (разреши установку из этого источника).
2. Вставь ссылку или отсканируй QR; при первом подключении система один раз спросит
   разрешение на VPN. Разрешение на уведомления нужно для постоянного уведомления сервиса
   с движком и состоянием.
3. Режим «по приложениям» (выбранные приложения целиком в туннель, остальные мимо) и
   исключения приложений — на экране «Правила»; домен можно добавить из share-меню.

### Движок CSQTT

Ему нужны хеши звонков VK (ссылка на звонок или хеш — экран «Движки → CSQTT») либо вход
через VK в том же месте; хеши и токен хранятся только на устройстве и в подписку не
попадают. Капчу VK приложение показывает само. Пока CSQTT ждёт действий пользователя,
автовыбор считает его не мёртвым, а ждущим — трафик идёт через AWG.

## Админу: хост, пользователи, подписка

CLI собирается `make ctl` (`core/target/release/braidctl`), данные живут в `state/`
(вне git): `hosts/*.toml`, `users/*.toml`, `rules/*.json`, `modes.json`, `secrets/`.
Любая команда с `--dry-run` только печатает, что сделала бы на хостах.

```
braidctl init                                 # раскладка state/
braidctl host add home --ssh user@host        # state/hosts/home.toml — заполни поля «впиши»
braidctl host import home                     # прочитать по SSH установленные AWG, csqtt-server, braid-subd
braidctl host setup home                      # поставить csqtt-server и braid-subd рядом с AWG (идемпотентно)
braidctl rules check                          # state/rules/*.json и modes.json
braidctl add-user me --host home --admin      # ключ AWG и пир, профиль CSQTT, токен, подписка
braidctl link me --qr                         # личная ссылка braid:// и QR
braidctl publish                              # собрать подписки и выложить на хосты (атомарно)
braidctl verify                               # сверить хеши локальной сборки и файлов на хостах
braidctl stats --host home --days 7           # сводка телеметрии: движок × сеть
braidctl revoke <имя>                         # снять пир, профиль CSQTT и файл подписки
braidctl import-amnezia <имя> <vpn://…> --host <id>   # существующий ключ Amnezia без перевыпуска
```

**`host import`, `host setup`, `add-user`, `revoke`, `publish` и `verify` ходят на реальный
хост по SSH.** `host setup` ставит `csqtt-server` из апстрима на закреплённом совместимом
коммите и `braid-subd` (`core/braid-subd/deploy/`), AWG не трогает; запуск на хосте — всегда
отдельное осознанное решение: сначала `--dry-run`, потом команда. Как устроен сервер CSQTT
и его пользователи — [`core/braidctl/docs/csqtt-server.md`](core/braidctl/docs/csqtt-server.md).

### Правила и режимы (`state/`)

`state/rules/<id>.json` — набор правил в формате `RuleSet` (`braid-proto`):

```json
{ "id": "cursor", "name": "Cursor",
  "domains": ["cursor.com", "*.cursor.com"], "ips": ["104.18.0.0/16"],
  "exclude_domains": [], "exclude_ips": [] }
```

`*.x.y` — любой поддомен на любой глубине; исключения побеждают включения; только IPv4.
`state/modes.json` — пресеты: `{ "modes": [ { "id", "name", "sets": [...], "exclude_domains",
"exclude_ips", "default" } ] }`, ровно один режим с `"default": true`. Правка списка →
`braidctl publish` → у клиентов при следующем обновлении подписки (при подключении и раз в
N часов) или по «Обновить сейчас».

Перенос списка Amnezia: `python3 tools/amnezia_to_rules.py archive/amnezia_x.json x "Имя"`.
Split-туннель для сервера zen: `python3 tools/gen_zen_vpnsplit.py <папка> <endpoint-IP>` —
читает `state/rules/*.json`; свои подсети «никогда в туннель» — `state/never.txt`.

## Разработка

Нужны Rust stable (`rustup`), Go 1.27 и Flutter 3.47 (`~/development/flutter`, Makefile
добавляет его в PATH). Xcode, Android SDK/NDK и JDK локально не нужны — `.app` и APK
собирает CI.

| Команда | Что делает |
|---|---|
| `make setup` | rustfmt и clippy, цели Rust, `flutter_rust_bridge_codegen`, `flutter pub get`, `go mod download` |
| `make check` | всё, что гоняет CI: версия одна везде, fmt/clippy/тесты Rust (ядро, мост, сквозной тест), кросс-проверка платформ под Linux/Windows/Android, go vet/test, flutter analyze/format/test, тесты `csqtt-core` |
| `make check-rust` / `check-cross` / `check-go` / `check-flutter` / `check-csqtt` / `check-version` | те же проверки по частям |
| `make codegen` | перегенерировать мост после правки `app/rust/src/api` |
| `make fmt` | отформатировать Rust и Dart |
| `make libawg`, `make libawg-android`, `make helper`, `make subd`, `make ctl` | компонентные сборки |
| `make macos`, `make apk` | `Braid.app` (universal, ad-hoc, zip) и APK в `dist/` — нужны Xcode и NDK с Java, делает CI |
| `make set-version V=x.y.z` | поменять версию везде; `make release` — проверить версию и свободный тег, собрать всё |

Один тест: `cd core && cargo test -p braid-runtime --test e2e` · `cd core && cargo test -p braid-core nat::` ·
`cd app && flutter test test/rules_test.dart` · `cd engines/awg-go && go test ./... -run TestPktTun`.

**Версия** одна на всё — файл `VERSION`; копии в `core/Cargo.toml`, `app/rust/Cargo.toml`
(её показывает «О программе» и по ней ядро сверяет `client.latest` подписки) и
`app/pubspec.yaml` сверяет `make check-version`. Выпуск: `make set-version V=0.2.0` →
коммит → `git tag v0.2.0 && git push origin v0.2.0` → `build.yml` сверяет тег с `VERSION`,
гоняет `make check`, собирает `.app` и APK и выкладывает Release. На каждый push в `main`
`check.yml` гоняет тесты, `build.yml` собирает артефакты прогона.

## Ограничения

- **Лицензия.** Движок CSQTT — вендоренное ядро `csqtt-core` под PolyForm Noncommercial
  1.0.0; лицензия действует на всё, что его линкует: сборки Braid с CSQTT — только для
  себя и своих, без продажи и коммерческого распространения.
- **Windows и Linux** заложены в архитектуру (`braid-platform`: заглушки, которые
  компилируются — `make check-cross`), сборок пока нет.
- **VK.** `client_id` в `VkOAuthConfig` (`app/lib/services/vk/vk_oauth.dart`) — приложение
  FOCSQ; для своей сборки заведи приложение VK и замени.
- **Вне репозитория.** Правила, хосты, пользователи и секреты (`state/`), архивы и запись
  автопилота (`.autopilot/`) в публичный репозиторий не входят — только код и документация.
- Туннель IPv4-only (AAAA доменов правил не отдаётся); автопереключения между хостами нет —
  защищённые приложения не должны менять IP; код PWDTT в сборку не входит.

## Карта репозитория

```
VERSION                   единственный источник версии приложения
app/                      Flutter-приложение (macOS, Android; Linux и Windows — заготовки)
  lib/                    экраны и сервисы; lib/src/rust — сгенерированный мост (в git)
  rust/                   braid_ffi — крейт моста flutter_rust_bridge
  rust_builder/           FFI-плагин с cargokit: Rust собирается внутри flutter build
  android/                сервис VPN (Kotlin), MethodChannel, unit-тесты (CI)
  macos/                  Runner: фаза «Embed braid-helper and libawg», braid:// в Info.plist
core/                     Cargo workspace
  braid-proto/            общие типы: схема подписки, ссылка braid://, события журнала
  braid-core/             роутер и всё платформонезависимое ядро (Core — один на процесс)
  braid-platform/         трейт Platform: macos/, android/, заглушки linux/ и windows/, memory для тестов
  braid-engine-awg/       движок AWG — обёртка над libawg
  braid-engine-csqtt/     движок CSQTT — обёртка над engines/csqtt-core (режим External)
  braid-runtime/          build_core: ядро с настоящими движками; JNI-мост Android; сквозной тест
  braid-helper/           root-помощник macOS (utun, маршруты, DNS)
  braid-subd/             сервер подписок и телеметрии (+ deploy/ для хоста)
  braidctl/               CLI админа (+ docs/csqtt-server.md)
engines/
  awg-go/                 libawg: cgo c-shared над amneziawg-go
  csqtt-core/             вендоренная копия csqtt-core с правками [BRAID]
tools/                    скрипты: генераторы списков, конвертер Amnezia → правила, split для zen
docs/                     architecture.md, adr/0001…0006
state/                    данные админа — ВНЕ git: rules/*.json, modes.json, hosts/, users/, secrets/
archive/                  дампы и исходные списки Amnezia — вне git
.github/workflows/        check.yml — тесты; build.yml — .app, APK, Release по тегу
```
