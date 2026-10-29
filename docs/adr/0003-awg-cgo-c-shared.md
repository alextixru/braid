# ADR 0003. AmneziaWG — cgo c-shared библиотека `libawg` с узким C-API

Статус: принято (2026-10-29).

## Контекст

Единственная живая реализация AmneziaWG 1.5/2.0/3.1 — `amneziawg-go` (Go). Ядро на Rust,
приложение на Flutter; на Android движок должен жить в процессе сервиса VPN и защищать свои
сокеты через `VpnService.protect`, а на macOS — работать без собственного интерфейса
(ADR 0001). Пользователь спрашивал, не нужно ли форкать ядро ради подписки на правила.

## Решение

`engines/awg-go` — Go-модуль поверх `amneziawg-go/v3` (пин `v3.1.20260828`), собираемый
как cgo c-shared (`make libawg` → `libawg.dylib` universal; `make libawg-android` →
`libawg.so` для arm64-v8a и x86_64, API 26, 16-КБ страницы). C-API намеренно узкое
(`engines/awg-go/awg.h`): `awg_new(uapi, mtu, callbacks, ctx)`, `awg_inject`,
`awg_set_config` / `awg_get_config`, `awg_bump_sockets`, `awg_close` и колбэки `tx`, `protect`,
`log`. Внутри — свой `tun.Device` поверх канала (пакеты вместо файла устройства) и bind с
`protect`. Rust-обёртка `core/braid-engine-awg` (`AwgEngine`, `UapiConfig`) собирает UAPI-текст
из `EngineSpec::Awg` подписки и линкует библиотеку (`build.rs` сам зовёт `make`, если сборки
нет; `BRAID_AWG_DIST` — готовая). Форк ядра не нужен: правила клиент забирает подпиской
независимо от ядра, ключи пользователей при смене списков не перевыпускаются.

## Последствия

- Нулевая цена IPC на пакет, одна схема на macOS и Android (как у wireguard-android и Amnezia).
- Go-рантайм в процессе приложения; на Android `libawg.so` лежит рядом с `libbraid_ffi.so`
  (`jniLibs` в `app/android/app/build.gradle.kts`), на macOS — в `Contents/Frameworks`.
- `awg_close` ждёт завершения колбэков — из колбэков его звать нельзя (документировано в `awg.h`).
- План Б без правок Go-кода — отдельный процесс-sidecar на десктопе.
- Нужен Go для сборки (локально и в CI); тесты Go — `make check-go`.
