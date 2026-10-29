# Архитектура Braid

Braid — один VPN-клиент для macOS и Android с несколькими движками к одному хосту:
AmneziaWG, CSQTT и слот под третий. Приложение владеет единственным tun-интерфейсом
и само решает, какой пакет идёт в туннель и каким движком; правила приходят подпиской
с хоста. Решения с обоснованием — в [`adr/`](adr/); здесь — как это разложено по коду.

## Слои и крейты

```
app/lib  (Flutter, Dart)        экраны, трей, WebView VK, QR — только через мост
   │  flutter_rust_bridge
app/rust  braid_ffi             зеркала Core::*, поток CoreState, строит ядро на macOS
   │
core/braid-runtime              build_core(platform, dir): ядро + фабрика движков; JNI-мост Android
   │                 ├── core/braid-engine-awg     AwgEngine над libawg (engines/awg-go, cgo c-shared)
   │                 └── core/braid-engine-csqtt   CsqttEngine над engines/csqtt-core (режим External)
core/braid-core                 Core, роутер, DNS-прокси и fake-IP, NAT, правила, движки, живость, политика, подписка, журнал
   │
core/braid-platform             трейт Platform: macos/ (клиент braid-helper), android/ (JNI), linux/ и windows/ (заглушки), memory (тесты)
core/braid-proto                словарь: Subscription, braid://, EngineSpec, RuleSet/Mode, LogEvent
```

Зависимости направлены вниз: ядро знает платформу через трейт, платформа ядро не знает
(JNI-входы Android, которым нужно ядро, поэтому живут в `braid-runtime/src/android.rs`).
Отдельно стоят бинари: `core/braid-helper` (root-помощник macOS), `core/braid-subd`
(сервер подписок и телеметрии на хосте), `core/braidctl` (CLI админа). Ядро одно на
процесс (`Core::global()`): на macOS его строит мост, на Android — сервис VPN через JNI,
окно Flutter берёт готовое.

## Модули ядра (`core/braid-core/src`)

| Модуль | Владеет | Главные типы |
|---|---|---|
| `core/` | фасад и автомат сессии: импорт ссылки, подключение, тики, перезапуски, состояние для UI | `Core`, `CoreDeps`, `CoreConfig`, `CoreError` |
| `tun/` | чтение и запись IP-пакетов в выданный fd, AF-заголовок macOS, подмена fd на Android | `TunIo` |
| `router/` | путь пакета: tun → решение → NAT → движок и обратно; пробы и DNS через туннель | `Router`, `RouterConfig`, `RouterEvent`, `NetResolver` |
| `dns/` | DNS-прокси на `10.255.0.1`, пул fake-IP `198.18.0.0/15`, DNS по TCP | `DnsProxy`, `FakeIpPool`, `Resolver`, `TcpDns` |
| `nat/` | таблица потоков и симметричный NAT с инкрементальными контрольными суммами, RST мёртвым потокам | `FlowTable`, `Decision`, `packet`, `checksum` |
| `rules/` | матчер доменов и подсетей: наборы подписки, режим, свои правила, исключения | `RuleBook`, `RulesState`, `UserRules`, `EnabledSets` |
| `engines/` | менеджер движков: запуск, тёплый запас, перезапуск упавшего 3 раза (2/5/15 с) | `EngineManager`, `EngineFactory`, `Guarded` |
| `health/` | живость: активная DNS-проба каждые 5 с, окно 30 с, смерть/деградация/стойло | `HealthMonitor`, `HealthConfig`, `Liveness` |
| `policy/` | чистый автомат выбора движка и память сети | `Policy`, `PolicyInputs`, `Decision`, `NetworkMemory` |
| `subscription/` | HTTPS-клиент с пином SPKI, ETag, хранилище последней копии, расписание обновления | `SubscriptionClient`, `SubscriptionManager`, `Store`, `PinVerifier` |
| `journal/` | JSONL-журнал админа с ротацией 30 дней и отправкой на хост пакетами | `Journal`, `TelemetryUploader` |
| `state/` | то, что видит интерфейс | `CoreState`, `SessionState`, `EngineView`, `RulesView`, `Banner` |
| `engine.rs` | контракт движка | `Engine`, `EngineIo`, `EngineHooks`, `EngineSnapshot`, `EngineEvent` |
| `testing/` | подделки для тестов без сети и привилегий | `NullEngine`, `LoopbackEngine`, `MemoryPlatform`, `FakeClock`, `MemoryStorage` |

## Путь пакета

Интерфейс получает фиксированный адрес `10.255.0.2`, собеседник и DNS — `10.255.0.1`
(`router::TUN_ADDRESS`, `PROXY_ADDRESS`). В интерфейс направляются только диапазон
fake-IP, подсети включённых наборов правил, адрес прокси и — на Android в режиме
«по приложениям» — трафик выбранных приложений; остальное до Braid не доходит.

1. **DNS.** Система спрашивает `10.255.0.1:53`. Домен из правил → `DnsProxy` выдаёт адрес
   из `198.18.0.0/15` (TTL 60 с) и заранее резолвит настоящий через активный движок
   (`NetResolver::tunnel`, резолвер движка или `probe.dns` подписки); AAAA для таких
   доменов не отдаётся (туннель IPv4-only). Домен не из правил → запрос уходит исходным
   резолверам системы напрямую (`NetResolver::direct`), в режиме «весь DNS через
   туннель» — тоже через движок.
2. **Решение.** Первый пакет потока (5-tuple) попадает в `Router::decide`: fake-IP → запись
   пула и настоящий адрес; подсеть правил → как есть; иначе `NotInRules`. Поток
   закрепляется за активным движком на всё время жизни (`Decision::{FakeIp, Real}`).
3. **NAT.** `FlowTable::outbound` подменяет источник на адрес движка в туннеле (AWG —
   `client_ip`, CSQTT — из `TUNCONF`) и назначение на настоящий адрес; движок получает
   готовый IP-пакет через `Engine::send`. Ответ приходит колбэком `EngineIo::inbound` →
   `FlowTable::inbound` восстанавливает адреса → `TunIo::write`.
4. **Пробы и запросы прокси** идут тем же NAT от имени `10.255.0.1` с временным портом;
   ответ ловится по порту (`Router::probe_via`, `register_pending`).
5. **Kill switch.** Живого движка нет — пакеты в fake-диапазон и подсети правил
   отбрасываются, DNS по правилам отвечает SERVFAIL; прямой трафик не затрагивается.
   «Отключить» снимает интерфейс — всё идёт напрямую.
6. **Смерть движка.** `Router::drop_engine` забывает его потоки и шлёт TCP-потокам RST,
   чтобы приложения переподключились через новый движок сразу, а не по таймауту.

## Живость и автомат политики

Каждый тик (1 с) `Core::tick` собирает пассивные сигналы (`EngineSnapshot`: байты,
возраст хендшейка, состояние) и активные пробы `HealthMonitor` — DNS-запрос случайного
имени через движок каждые 5 с (30 с при погашенном экране). Вердикт `Liveness`:

| Вердикт | Условие (`HealthConfig`) |
|---|---|
| `Dead(NoReply)` | три пробы подряд без ответа (таймаут 4 с) |
| `Dead(HandshakeStale)` | хендшейк AWG старше 180 с |
| `Dead(Engine)` | движок сам сообщил `Dead` или упал и не перезапустился |
| `Degraded(Loss)` | потери > 50 % за окно 30 с (не меньше трёх завершённых проб) |
| `Degraded(Rtt)` | медианный RTT > 2 с |
| `Degraded(Stall)` | tx растёт, rx стоит 15 с |
| `WaitingUser` | капча, нужны хеши или вход VK — не смерть |

`Policy::decide` получает приоритет движков из подписки, вердикты, активный движок и режим
(`Auto` или `Pinned`) и возвращает `Action::{Stay, Switch{to, reason}, NoneAlive}` плюс кого
держать тёплым (следующий по приоритету, подключён без трафика):

- мёртвый активный → немедленно на живой тёплый (`reason: Dead`), без ограничений частоты;
- деградировавший активный → на здоровый тёплый (`Degraded`/`Stall`), не чаще раза в 2 мин;
- возврат на приоритетный — после 60 с непрерывно живых проб (`Priority`);
- `Pinned` отключает автоматику: мёртвый закреплённый не подменяется, статус честный;
- старт сессии — `Policy::initial`: закреплённый; в известной сети — последний рабочий по
  `NetworkMemory` (движок с тремя неудачами подряд пропускается); иначе первый по приоритету.

Каждое переключение — событие `Switch{from, to, reason, metrics}` в журнале (у админа),
уведомление ОС при включённой настройке и RST потокам мёртвого движка.

## Платформы

- **macOS** (`braid-platform/src/macos`, `braid-helper`): utun создаёт root-помощник;
  приложение получает fd по UDS (`SCM_RIGHTS`), помощник ставит адрес, маршруты и DNS,
  меняет маршруты на лету и откатывает всё при `down` или обрыве сокета, при старте чистит
  хвосты прошлой сессии. Запуск — `sudo -n` по правилу `/etc/sudoers.d/braid` на копию
  помощника в `/Library/PrivilegedHelperTools` (ставится с согласия пользователя), иначе
  системный диалог пароля. Помощник и `libawg.dylib` лежат внутри `Braid.app`.
- **Android** (`braid-platform/src/android`, `app/android`): `BraidVpnService` (foreground,
  always-on, `BootReceiver`) делает `establish()` и отдаёт fd ядру через JNI; смена подсетей
  или списка приложений — новый `establish()` с бесшовной подменой fd (`TunIo::swap_fd`);
  сокеты движков защищаются `VpnService.protect`. Ядро живёт в процессе сервиса.
- **Linux, Windows** (`linux.rs`, `windows.rs`): реализации `Platform`, которые собираются и
  честно отвечают `Unsupported` — заложено для следующего захода (`make check-cross`).

## Подписка и сервер

Личная ссылка `braid://base64url({v, url, token, pin})` → `GET <url>/sub/<token>.json` по
HTTPS с проверкой SPKI-пина сертификата хоста вместо CA; `ETag`/`If-None-Match`, повтор при
подключении и раз в N часов. Подписка несёт хост, движки с параметрами, наборы правил и
режимы, `probe.dns`, `client.latest` и — только у админа — `telemetry`. На хосте её раздаёт
`braid-subd` (статические файлы, `POST /telemetry`), собирает и выкладывает `braidctl publish`
из `state/` админа; протокольные серверы не меняются.

## Проверка

Все тесты — без сети и привилегий (`make check`): роутер и ядро на `MemoryPlatform`
(socketpair вместо tun), `LoopbackEngine`/`NullEngine`, `FakeClock`; подписка — против
`braid-subd`, поднятого в процессе на `127.0.0.1`. Сквозной тест
`core/braid-runtime/tests/e2e.rs` проводит ядро от импорта ссылки до переключения движка
по смерти: DNS домена из набора → fake-IP → UDP через роутер и обратно → три потерянные
пробы → `Switch{reason: Dead}` в журнале и телеметрии, RST старому TCP-потоку, новые
потоки во втором движке.
